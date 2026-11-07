//! Evaluating a workbook's formulas against a projection of the model.

use std::io::Write;

use jade_xlsx_calc::evaluate;
use jade_xlsx_reader::{read_xlsx, CellRef, CellValue};
use zip::write::SimpleFileOptions;

#[test]
fn legacy_cse_truncates_larger_results_and_repeats_scalar_results() {
    let xml = sheet(
        r#"<row r="1"><c r="A1"><f t="array" ref="A1:A2">{1;2;3}</f><v>1</v></c><c r="B1"><f t="array" ref="B1:B3">7</f><v>7</v></c><c r="C1"><f>A3+B3</f></c></row><row r="2"><c r="A2"><v>2</v></c><c r="B2"><v>7</v></c></row><row r="3"><c r="A3"><v>99</v></c><c r="B3"><v>7</v></c></row>"#,
    );
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    assert_eq!(computed(&result, 0, "C1"), Some(&CellValue::Number(106.0)));
    for (anchor, range, values) in [("A1", "A1:A2", vec![2.0]), ("B1", "B1:B3", vec![7.0, 7.0])] {
        let array = result
            .spills
            .iter()
            .find(|spill| spill.anchor == CellRef::parse(anchor).unwrap())
            .unwrap();
        assert_eq!(
            array.range,
            jade_xlsx_reader::CellRange::parse(range).unwrap()
        );
        assert_eq!(
            array
                .cells
                .iter()
                .map(|cell| cell.value.clone())
                .collect::<Vec<_>>(),
            values
                .into_iter()
                .map(CellValue::Number)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn unsupported_legacy_arrays_preserve_cached_followers_without_eager_expansion() {
    for (formula, range) in [("FUTUREFUNC()", "A1:A3"), ("1", "A1:B1048576")] {
        let xml = sheet(&format!(
            r#"<row r="1"><c r="A1"><f t="array" ref="{range}">{formula}</f><v>10</v></c><c r="C1"><f>A2+1</f></c></row><row r="2"><c r="A2"><v>20</v></c></row>"#
        ));
        let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
        assert!(!result.skipped.is_empty(), "{range}");
        assert_eq!(
            computed(&result, 0, "C1"),
            Some(&CellValue::Number(21.0)),
            "{range}"
        );
        assert!(result.spills.is_empty());
    }
}

#[test]
fn legacy_cse_output_keeps_its_declared_range_when_the_result_is_shorter() {
    let xml = sheet(
        r#"<row r="1"><c r="A1"><f t="array" ref="A1:A3">{1;2}</f><v>1</v></c><c r="B1"><f>A3</f></c></row><row r="2"><c r="A2"><v>2</v></c></row><row r="3"><c r="A3" t="e"><v>#N/A</v></c></row>"#,
    );
    let book = read_xlsx(&workbook(&xml, None, false)).unwrap();
    let result = evaluate(&book).unwrap();
    assert!(result.skipped.is_empty());
    let array = result
        .spills
        .iter()
        .find(|spill| spill.anchor == CellRef::new(0, 0))
        .unwrap();
    assert_eq!(
        array.range,
        jade_xlsx_reader::CellRange::parse("A1:A3").unwrap()
    );
    assert_eq!(
        array
            .cells
            .iter()
            .find(|cell| cell.reference == CellRef::new(0, 2))
            .map(|cell| &cell.value),
        Some(&CellValue::Error("#N/A".into()))
    );
    assert_eq!(
        computed(&result, 0, "B1"),
        Some(&CellValue::Error("#N/A".into()))
    );
}

#[test]
fn legacy_array_formula_text_keeps_cse_braces_without_bracing_unknown_arrays() {
    let xml = sheet(
        r#"<row r="1"><c r="A1"><f t="array" ref="A1">SUM(1, 2)</f><v>3</v></c><c r="B1" cm="1"><f t="array" ref="B1">SUM(1, 2)</f><v>3</v></c><c r="C1"><f>FORMULATEXT(A1)</f></c><c r="D1"><f>FORMULATEXT(B1)</f></c></row>"#,
    );
    let book = read_xlsx(&workbook(&xml, None, false)).unwrap();
    for _ in 0..2 {
        let result = evaluate(&book).unwrap();
        assert!(result.skipped.is_empty());
        assert_eq!(
            computed(&result, 0, "C1"),
            Some(&CellValue::Text("{=SUM(1, 2)}".into()))
        );
        assert_eq!(
            computed(&result, 0, "D1"),
            Some(&CellValue::Text("=SUM(1, 2)".into()))
        );
    }
}

#[test]
fn parsed_formula_source_spelling_survives_cache_and_kept_recalculation() {
    let xml = sheet(
        r#"<row r="1"><c r="A1"><v>2</v></c><c r="B1"><f>SUM(A1, 3)</f><v>5</v></c><c r="C1"><f>FORMULATEXT(B1)</f></c><c r="D1"><f>_xlfn.XLOOKUP(A1, A1:A1, B1:B1)</f><v>5</v></c><c r="E1"><f>FORMULATEXT(D1)</f></c></row>"#,
    );
    let mut book = read_xlsx(&workbook(&xml, None, false)).unwrap();
    for _ in 0..2 {
        let result = evaluate(&book).unwrap();
        assert!(result.skipped.is_empty());
        assert_eq!(
            computed(&result, 0, "C1"),
            Some(&CellValue::Text("=SUM(A1, 3)".into()))
        );
        assert_eq!(
            computed(&result, 0, "E1"),
            Some(&CellValue::Text("=XLOOKUP(A1, A1:A1, B1:B1)".into()))
        );
    }
    let manual = Default::default();
    let (_, session) = jade_xlsx_calc::evaluate_keeping(&book, "UTC", &manual).unwrap();
    let mut session = session.expect("scalar formulas can keep their calculation model");
    book.sheets[0]
        .cell_at_mut(CellRef::new(0, 0))
        .unwrap()
        .value = CellValue::Number(4.0);
    let result = jade_xlsx_calc::reevaluate(
        &mut session,
        &book,
        "UTC",
        &manual,
        &[(0, CellRef::new(0, 0), CellValue::Number(4.0))],
    )
    .unwrap()
    .unwrap();
    assert_eq!(computed(&result, 0, "B1"), Some(&CellValue::Number(7.0)));
    assert_eq!(
        computed(&result, 0, "C1"),
        Some(&CellValue::Text("=SUM(A1, 3)".into()))
    );
    book.sheets[0]
        .cell_at_mut(CellRef::new(1, 0))
        .unwrap()
        .formula = Some("SUM(A1,  3)".into());
    let result = evaluate(&book).unwrap();
    assert_eq!(
        computed(&result, 0, "C1"),
        Some(&CellValue::Text("=SUM(A1,  3)".into()))
    );
}

#[test]
fn conditional_aggregates_parse_numeric_criterion_spaces_without_trimming_text() {
    let formulas = [
        r#"COUNTIF(A1:A6,"< 20000")"#,
        r#"COUNTIFS(A1:A6,"< 20000")"#,
        r#"SUMIF(A1:A6,"< 20000",B1:B6)"#,
        r#"SUMIFS(B1:B6,A1:A6,"< 20000")"#,
        r#"AVERAGEIF(A1:A6,"< 20000",B1:B6)"#,
        r#"AVERAGEIFS(B1:B6,A1:A6,"< 20000")"#,
        r#"MAXIFS(B1:B6,A1:A6,"< 20000")"#,
        r#"MINIFS(B1:B6,A1:A6,"< 20000")"#,
        r#"COUNTIF(C1:C2,"= red ")"#,
        r#"COUNTIF(C1:C2,"= * ")"#,
    ];
    let data = [10000, 15000, 19000, 17000, 20000, 30000];
    let mut rows = String::new();
    for (index, formula) in formulas.iter().enumerate() {
        let r = index + 1;
        rows.push_str(&format!("<row r=\"{r}\">"));
        if let Some(number) = data.get(index) {
            rows.push_str(&format!(
                "<c r=\"A{r}\"><v>{number}</v></c><c r=\"B{r}\"><v>{r}</v></c>"
            ));
        }
        if index < 2 {
            let text = if index == 0 { " red " } else { "red" };
            rows.push_str(&format!(
                "<c r=\"C{r}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{text}</t></is></c>"
            ));
        }
        let formula = formula.replace('&', "&amp;").replace('<', "&lt;");
        rows.push_str(&format!("<c r=\"D{r}\"><f>{formula}</f></c></row>"));
    }
    let result = evaluate(&read_xlsx(&workbook(&sheet(&rows), None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty(), "{:?}", result.skipped);
    for (index, expected) in [4.0, 4.0, 10.0, 10.0, 2.5, 2.5, 4.0, 1.0, 1.0, 1.0]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            computed(&result, 0, &format!("D{}", index + 1)),
            Some(&CellValue::Number(expected)),
            "{}",
            formulas[index]
        );
    }
}

#[test]
fn financial_scalar_functions_broadcast_values_and_domain_errors() {
    for (values, expected, errors, invalid, singleton) in [
        (
            "SLN({100;190},10,{3,6})",
            "{30,15;60,30}",
            "SLN({#N/A;100},10,{0,3})",
            "{#N/A,#N/A;#DIV/0!,30}",
            "SLN({100},10,3)",
        ),
        (
            "SYD({100;190},10,3,{1,2})",
            "{45,30;90,60}",
            "SYD({#N/A;100},10,3,{0,1})",
            "{#N/A,#N/A;#NUM!,45}",
            "SYD({100},10,3,1)",
        ),
        (
            "EFFECT({0.1;0.2},{1,2})",
            "{0.1,0.1025;0.2,0.21}",
            "EFFECT({#N/A;0.1},{0,2})",
            "{#N/A,#N/A;#NUM!,0.1025}",
            "EFFECT({0.1},2)",
        ),
        (
            "NOMINAL({0.21;0.44},{1,2})",
            "{0.21,0.2;0.44,0.4}",
            "NOMINAL({#N/A;0.21},{0,2})",
            "{#N/A,#N/A;#NUM!,0.2}",
            "NOMINAL({0.21},2)",
        ),
        (
            "RRI({1;2},100,{400,1600})",
            "{3,15;1,3}",
            "RRI({#N/A;2},{0,100},400)",
            "{#N/A,#N/A;#NUM!,1}",
            "RRI({2},100,400)",
        ),
        (
            "PDURATION({1;3},100,{400,1600})",
            "{2,4;1,2}",
            "PDURATION({#N/A;1},100,{0,400})",
            "{#N/A,#N/A;#NUM!,2}",
            "PDURATION({1},100,400)",
        ),
    ] {
        let values = format!("ARRAYTOTEXT(ROUND({values},8),1)");
        let errors = format!("ARRAYTOTEXT(ROUND({errors},8),1)");
        let singleton = format!("TYPE({singleton})");
        assert_formulas(
            &[&values, &errors, &singleton],
            &[
                CellValue::Text(expected.into()),
                CellValue::Text(invalid.into()),
                CellValue::Number(64.0),
            ],
            false,
        );
    }
    assert_formulas(
        &[
            "ARRAYTOTEXT(ROUND(EFFECT({TRUE,0.1},2.9),8),1)",
            "ARRAYTOTEXT(ROUND(NOMINAL({TRUE,0.21},2.9),8),1)",
        ],
        &[
            CellValue::Text("{#VALUE!,0.1025}".into()),
            CellValue::Text("{#VALUE!,0.2}".into()),
        ],
        false,
    );
}

#[test]
fn dollar_conversions_broadcast_values_denominators_and_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(ROUND(DOLLARDE({1.02;1.04},{16,32}),8),1)",
            "ARRAYTOTEXT(ROUND(DOLLARFR({1.125;1.25},{16,32}),8),1)",
            "ARRAYTOTEXT(DOLLARDE(1.02,{-1,0,0.9}),1)",
            "ARRAYTOTEXT(DOLLARFR(1.125,{-1,0,0.9}),1)",
            "ARRAYTOTEXT(DOLLARDE({#N/A;1.02},{16,#DIV/0!}),1)",
            "ARRAYTOTEXT(DOLLARFR({#N/A;1.125},{16,#DIV/0!}),1)",
            "ARRAYTOTEXT(DOLLARDE({TRUE,1.02},16),1)",
            "ARRAYTOTEXT(DOLLARFR({TRUE,1.125},16),1)",
            "TYPE(DOLLARDE({1.02},16))",
            "TYPE(DOLLARFR({1.125},16))",
        ],
        &[
            CellValue::Text("{1.125,1.0625;1.25,1.125}".into()),
            CellValue::Text("{1.02,1.04;1.04,1.08}".into()),
            CellValue::Text("{#NUM!,#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{#NUM!,#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{#N/A,#N/A;1.125,#DIV/0!}".into()),
            CellValue::Text("{#N/A,#N/A;1.02,#DIV/0!}".into()),
            CellValue::Text("{#VALUE!,1.125}".into()),
            CellValue::Text("{#VALUE!,1.02}".into()),
            CellValue::Number(64.0),
            CellValue::Number(64.0),
        ],
        false,
    );
}

#[test]
fn convert_broadcasts_values_and_unit_axes_with_per_element_errors() {
    assert_formulas(
        &[
            r#"ARRAYTOTEXT(CONVERT({1;2},"m",{"cm","km"}),1)"#,
            r#"ARRAYTOTEXT(CONVERT(1,{"m";"g"},{"cm","kg"}),1)"#,
            r#"ARRAYTOTEXT(ROUND(CONVERT({68;32},"F",{"C","K"}),2),1)"#,
            r#"ARRAYTOTEXT(CONVERT({1;#N/A},"m",{"cm",#DIV/0!}),1)"#,
            r#"ARRAYTOTEXT(CONVERT({1,2},"m",{"M","日本語"}),1)"#,
            r#"TYPE(CONVERT({1},"m","cm"))"#,
            r#"ARRAYTOTEXT(CONVERT({1;2},"kibyte",{"byte","bit"}),1)"#,
            r#"ARRAYTOTEXT(CONVERT({1;2},"kiB",{"byte","bit"}),1)"#,
        ],
        &[
            CellValue::Text("{100,0.001;200,0.002}".into()),
            CellValue::Text("{100,#N/A;#N/A,0.001}".into()),
            CellValue::Text("{20,293.15;0,273.15}".into()),
            CellValue::Text("{100,#DIV/0!;#N/A,#N/A}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Number(64.0),
            CellValue::Text("{1024,8192;2048,16384}".into()),
            CellValue::Text("{#N/A,#N/A;#N/A,#N/A}".into()),
        ],
        false,
    );
}

#[test]
fn convert_celsius_uses_the_kelvin_offset_in_both_directions() {
    assert_formulas(
        &[
            r#"ROUND(CONVERT(68,"F","C"),10)"#,
            r#"ROUND(CONVERT(273.15,"K","C"),10)"#,
            r#"ROUND(CONVERT(20,"C","C"),10)"#,
            r#"ROUND(CONVERT(CONVERT(-40,"C","F"),"F","C"),10)"#,
            r#"ROUND(CONVERT(16,"Reau","cel"),10)"#,
            r#"ROUND(CONVERT(527.67,"Rank","C"),10)"#,
        ],
        &[
            CellValue::Number(20.0),
            CellValue::Number(0.0),
            CellValue::Number(20.0),
            CellValue::Number(-40.0),
            CellValue::Number(20.0),
            CellValue::Number(20.0),
        ],
        false,
    );
}

#[test]
fn complex_aggregates_accept_one_through_255_scalar_arguments() {
    let ones = std::iter::repeat_n("1", 255).collect::<Vec<_>>().join(",");
    for (function, expected, maximum) in [("IMSUM", "6+2i", "255"), ("IMPRODUCT", "6+12i", "1")] {
        let single = format!(r#"{function}("1+2j")"#);
        let three = format!(r#"{function}("1+2i",2,3)"#);
        let full = format!("{function}({ones})");
        let source_error = format!("{function}(1,2,#N/A)");
        assert_formulas(
            &[&single, &three, &full, &source_error],
            &[
                CellValue::Text("1+2j".into()),
                CellValue::Text(expected.into()),
                CellValue::Text(maximum.into()),
                CellValue::Error("#N/A".into()),
            ],
            false,
        );
    }
}

#[test]
fn complex_transcendentals_and_binary_operations_broadcast() {
    for (function, input, expected) in [
        ("IMCOS", 0, "1"),
        ("IMCOSH", 0, "1"),
        ("IMCOT", 1, "0.642093"),
        ("IMCSC", 1, "1.188395"),
        ("IMCSCH", 1, "0.850918"),
        ("IMEXP", 0, "1"),
        ("IMLN", 1, "0"),
        ("IMLOG10", 10, "1"),
        ("IMLOG2", 2, "1"),
        ("IMSEC", 0, "1"),
        ("IMSECH", 0, "1"),
        ("IMSIN", 0, "0"),
        ("IMSINH", 0, "0"),
        ("IMSQRT", 4, "2"),
        ("IMTAN", 0, "0"),
    ] {
        let values = format!("ARRAYTOTEXT(ROUND(IMREAL({function}({{#N/A;{input}}})),6),1)");
        let text_type = format!("TYPE(INDEX({function}({{#N/A;{input}}}),2,1))");
        let singleton = format!("TYPE({function}({{{input}}}))");
        let invalid = format!(r#"ARRAYTOTEXT({function}({{"invalid",#DIV/0!}}),1)"#);
        assert_formulas(
            &[&values, &text_type, &singleton, &invalid],
            &[
                CellValue::Text(format!("{{#N/A;{expected}}}")),
                CellValue::Number(2.0),
                CellValue::Number(64.0),
                CellValue::Text("{#NUM!,#DIV/0!}".into()),
            ],
            false,
        );
    }
    assert_formulas(
        &[
            r#"ARRAYTOTEXT(IMDIV({"2+4i";#N/A},{1,2}),1)"#,
            r#"ARRAYTOTEXT(IMSUB({"2+4i";#N/A},{1,2}),1)"#,
            r#"ARRAYTOTEXT(IMPOWER({2;#N/A},{2,3}),1)"#,
            r#"ARRAYTOTEXT(IMDIV({"i","j"},"i"),1)"#,
            r#"ARRAYTOTEXT(IMPOWER(2,{TRUE,#N/A}),1)"#,
        ],
        &[
            CellValue::Text(r#"{"2+4i","1+2i";#N/A,#N/A}"#.into()),
            CellValue::Text(r#"{"1+4i","4i";#N/A,#N/A}"#.into()),
            CellValue::Text(r#"{"4","8";#N/A,#N/A}"#.into()),
            CellValue::Text(r#"{"1",#VALUE!}"#.into()),
            CellValue::Text("{#VALUE!,#N/A}".into()),
        ],
        false,
    );
}

#[test]
fn complex_components_broadcast_typed_results_and_source_errors() {
    for (function, expected) in [
        ("IMABS", "5"),
        ("IMREAL", "3"),
        ("IMAGINARY", "4"),
        ("IMCONJUGATE", r#""3-4j""#),
    ] {
        let formula = format!(r#"ARRAYTOTEXT({function}({{#N/A;"3+4j"}}),1)"#);
        let singleton = format!(r#"TYPE({function}({{"3+4j"}}))"#);
        let invalid = format!(r#"ARRAYTOTEXT({function}({{"invalid",#DIV/0!}}),1)"#);
        assert_formulas(
            &[&formula, &singleton, &invalid],
            &[
                CellValue::Text(format!("{{#N/A;{expected}}}")),
                CellValue::Number(64.0),
                CellValue::Text("{#NUM!,#DIV/0!}".into()),
            ],
            false,
        );
    }
    assert_formulas(
        &[
            r#"ARRAYTOTEXT(COMPLEX({3;#N/A},{4,-4},"j"),1)"#,
            r#"ARRAYTOTEXT(COMPLEX(3,4,{"i","j","I"}),1)"#,
            r#"ARRAYTOTEXT(ROUND(IMARGUMENT({"i","-i",0,#N/A}),6),1)"#,
            "TYPE(COMPLEX({3},4))",
        ],
        &[
            CellValue::Text(r#"{"3+4j","3-4j";#N/A,#N/A}"#.into()),
            CellValue::Text(r#"{"3+4i","3+4j",#VALUE!}"#.into()),
            CellValue::Text("{1.570796,-1.570796,#DIV/0!,#N/A}".into()),
            CellValue::Number(64.0),
        ],
        false,
    );
}

#[test]
fn bessel_functions_broadcast_values_orders_and_errors() {
    for (function, expected) in [
        ("BESSELI", "{1.266066,0.565159;2.279585,1.590637}"),
        ("BESSELJ", "{0.765198,0.440051;0.223891,0.576725}"),
        ("BESSELK", "{0.421024,0.601907;0.113894,0.139866}"),
        ("BESSELY", "{0.088257,-0.781213;0.510376,-0.107032}"),
    ] {
        let values = format!("ARRAYTOTEXT(ROUND({function}({{1;2}},{{0,1}}),6),1)");
        let errors = format!("ARRAYTOTEXT({function}({{#N/A;#DIV/0!}},{{0,1}}),1)");
        let invalid = format!("ARRAYTOTEXT({function}({{1,TRUE}},-1),1)");
        let singleton = format!("TYPE({function}({{1}},0))");
        assert_formulas(
            &[&values, &errors, &invalid, &singleton],
            &[
                CellValue::Text(expected.into()),
                CellValue::Text("{#N/A,#N/A;#DIV/0!,#DIV/0!}".into()),
                CellValue::Text("{#NUM!,#VALUE!}".into()),
                CellValue::Number(64.0),
            ],
            false,
        );
    }
}

#[test]
fn engineering_radix_conversions_broadcast_inputs_and_places() {
    for (function, result) in [
        ("BIN2DEC", "2"),
        ("BIN2HEX", r#""2""#),
        ("BIN2OCT", r#""2""#),
        ("DEC2BIN", r#""1010""#),
        ("DEC2HEX", r#""A""#),
        ("DEC2OCT", r#""12""#),
        ("HEX2BIN", r#""10000""#),
        ("HEX2DEC", "16"),
        ("HEX2OCT", r#""20""#),
        ("OCT2BIN", r#""1000""#),
        ("OCT2DEC", "8"),
        ("OCT2HEX", r#""8""#),
    ] {
        let formula = format!(r#"ARRAYTOTEXT({function}({{#N/A;"10"}}),1)"#);
        let singleton = format!(r#"TYPE({function}({{"10"}}))"#);
        assert_formulas(
            &[&formula, &singleton],
            &[
                CellValue::Text(format!("{{#N/A;{result}}}")),
                CellValue::Number(64.0),
            ],
            false,
        );
    }
    assert_formulas(
        &[
            "ARRAYTOTEXT(BIN2HEX({10;11},{1,3}),1)",
            "ARRAYTOTEXT(DEC2BIN({2;3},{1,3}),1)",
            r#"ARRAYTOTEXT(HEX2OCT({"10";"11"},{1,3}),1)"#,
            r#"ARRAYTOTEXT(OCT2HEX({"10";"11"},{1,3}),1)"#,
        ],
        &[
            CellValue::Text(r#"{"2","002";"3","003"}"#.into()),
            CellValue::Text(r#"{#NUM!,"010";#NUM!,"011"}"#.into()),
            CellValue::Text(r#"{#NUM!,"020";#NUM!,"021"}"#.into()),
            CellValue::Text(r#"{"8","008";"9","009"}"#.into()),
        ],
        false,
    );
}

#[test]
fn decimal_broadcasts_text_radices_and_accepts_wide_integers() {
    assert_formulas(
        &[
            r#"DECIMAL("FFFFFFFFFFFFFFFF",16)/2^64"#,
            r#"DECIMAL("18446744073709551615",10)/2^64"#,
            r#"DECIMAL(REPT("F",255),16)/2^1020"#,
            r#"DECIMAL(REPT("Z",255),36)"#,
            r#"DECIMAL(REPT("0",256),2)"#,
            r#"ARRAYTOTEXT(DECIMAL({"10";"ff"},{2,16}),1)"#,
            r#"ARRAYTOTEXT(DECIMAL({"10";#N/A},{2,#DIV/0!}),1)"#,
            r#"TYPE(DECIMAL({"10"},2))"#,
            r#"DECIMAL("-FF",16)"#,
            r#"DECIMAL("ff",16.9)"#,
        ],
        &[
            CellValue::Number(1.0),
            CellValue::Number(1.0),
            CellValue::Number(1.0),
            CellValue::Error("#NUM!".into()),
            CellValue::Error("#NUM!".into()),
            CellValue::Text("{2,16;#VALUE!,255}".into()),
            CellValue::Text("{2,#DIV/0!;#N/A,#N/A}".into()),
            CellValue::Number(64.0),
            CellValue::Number(-255.0),
            CellValue::Number(255.0),
        ],
        false,
    );
}

#[test]
fn base_broadcasts_radices_padding_and_enforces_documented_bounds() {
    assert_formulas(
        &[
            "BASE(1,2,256)",
            "BASE(1,2,1E100)",
            "BASE(9007199254740992,16)",
            "BASE(2^53-1,16)",
            "ARRAYTOTEXT(BASE({10;15},{2,16}),1)",
            "ARRAYTOTEXT(BASE({1;2},2,{0,4}),1)",
            "LEN(BASE(1,2,255))",
            "BASE(15.9,16.9,4.9)",
            "ARRAYTOTEXT(BASE({#N/A;1},{2,#DIV/0!}),1)",
            "TYPE(BASE({1},2))",
        ],
        &[
            CellValue::Error("#NUM!".into()),
            CellValue::Error("#NUM!".into()),
            CellValue::Error("#NUM!".into()),
            CellValue::Text("1FFFFFFFFFFFFF".into()),
            CellValue::Text(r#"{"1010","A";"1111","F"}"#.into()),
            CellValue::Text(r#"{"1","0001";"10","0010"}"#.into()),
            CellValue::Number(255.0),
            CellValue::Text("000F".into()),
            CellValue::Text(r#"{#N/A,#N/A;"1",#DIV/0!}"#.into()),
            CellValue::Number(64.0),
        ],
        false,
    );
}

#[test]
fn bit_operations_broadcast_and_detect_wide_shift_overflow() {
    assert_formulas(
        &[
            "BITLSHIFT(140737488355328,17)",
            "BITRSHIFT(140737488355328,-17)",
            "ARRAYTOTEXT(BITAND({2;3},{1,2}),1)",
            "ARRAYTOTEXT(BITOR({2;3},{1,2}),1)",
            "ARRAYTOTEXT(BITXOR({2;3},{1,2}),1)",
            "ARRAYTOTEXT(BITLSHIFT({2;3},{-1,1}),1)",
            "ARRAYTOTEXT(BITRSHIFT({2;3},{-1,1}),1)",
            "ARRAYTOTEXT(BITLSHIFT({0;1},{47,48,53,54}),1)",
            "ARRAYTOTEXT(BITRSHIFT({281474976710655;3},{53,-53}),1)",
            "ARRAYTOTEXT(BITAND({-1,0.5,281474976710656,#N/A},1),1)",
            "ARRAYTOTEXT(BITXOR({1;2},{#N/A,#DIV/0!}),1)",
            "TYPE(BITAND({1},1))+TYPE(BITOR({1},1))+TYPE(BITXOR({1},1))+TYPE(BITLSHIFT({1},1))+TYPE(BITRSHIFT({1},1))",
        ],
        &[
            CellValue::Error("#NUM!".into()),
            CellValue::Error("#NUM!".into()),
            CellValue::Text("{0,2;1,2}".into()),
            CellValue::Text("{3,2;3,3}".into()),
            CellValue::Text("{3,0;2,1}".into()),
            CellValue::Text("{1,4;1,6}".into()),
            CellValue::Text("{4,1;6,1}".into()),
            CellValue::Text("{0,0,0,#NUM!;140737488355328,#NUM!,#NUM!,#NUM!}".into()),
            CellValue::Text("{0,#NUM!;0,#NUM!}".into()),
            CellValue::Text("{#NUM!,#NUM!,#NUM!,#N/A}".into()),
            CellValue::Text("{#N/A,#DIV/0!;#N/A,#DIV/0!}".into()),
            CellValue::Number(320.0),
        ],
        false,
    );
}

#[test]
fn engineering_comparisons_broadcast_operands_defaults_and_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(DELTA({-1;0;1},{0,1}),1)",
            "ARRAYTOTEXT(GESTEP({-1;0;1},{0,1}),1)",
            "ARRAYTOTEXT(DELTA({0,1,-1}),1)",
            "ARRAYTOTEXT(GESTEP({0,1,-1},),1)",
            "ARRAYTOTEXT(DELTA({0;1},{#N/A,#DIV/0!}),1)",
            "ARRAYTOTEXT(GESTEP({0;1},{#N/A,#DIV/0!}),1)",
            "ARRAYTOTEXT(DELTA({\"0\",TRUE},0),1)",
            "ARRAYTOTEXT(GESTEP(0,{\"0\",FALSE}),1)",
            "TYPE(DELTA({0}))+TYPE(GESTEP({0}))",
        ],
        &[
            CellValue::Text("{0,0;1,0;0,1}".into()),
            CellValue::Text("{0,0;1,0;1,1}".into()),
            CellValue::Text("{1,0,0}".into()),
            CellValue::Text("{1,1,0}".into()),
            CellValue::Text("{#N/A,#DIV/0!;#N/A,#DIV/0!}".into()),
            CellValue::Text("{#N/A,#DIV/0!;#N/A,#DIV/0!}".into()),
            CellValue::Text("{1,#VALUE!}".into()),
            CellValue::Text("{1,#VALUE!}".into()),
            CellValue::Number(128.0),
        ],
        false,
    );
}

#[test]
fn error_functions_broadcast_bounds_and_preserve_typed_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(ROUND(ERF({-1,0;1,#N/A}),8),1)",
            "ARRAYTOTEXT(ROUND(ERF({-1;0},{0,1}),8),1)",
            "ARRAYTOTEXT(ROUND(ERF.PRECISE({-1,0,1}),8),1)",
            "ARRAYTOTEXT(ROUND(ERFC({-1,0,1}),8),1)",
            "ARRAYTOTEXT(ROUND(ERFC.PRECISE({-1,0,1}),8),1)",
            "ARRAYTOTEXT(ERF({0;1},{#N/A,#DIV/0!}),1)",
            "ARRAYTOTEXT(ERF.PRECISE({#N/A,TRUE,\"0\"}),1)",
            "ARRAYTOTEXT(ERFC.PRECISE({#DIV/0!,FALSE,\"0\"}),1)",
            "TYPE(ERF({0},1))+TYPE(ERF.PRECISE({0}))+TYPE(ERFC({0}))+TYPE(ERFC.PRECISE({0}))",
        ],
        &[
            CellValue::Text("{-0.84270079,0;0.84270079,#N/A}".into()),
            CellValue::Text("{0.84270079,1.68540159;0,0.84270079}".into()),
            CellValue::Text("{-0.84270079,0,0.84270079}".into()),
            CellValue::Text("{1.84270079,1,0.15729921}".into()),
            CellValue::Text("{1.84270079,1,0.15729921}".into()),
            CellValue::Text("{#N/A,#DIV/0!;#N/A,#DIV/0!}".into()),
            CellValue::Text("{#N/A,#VALUE!,0}".into()),
            CellValue::Text("{#DIV/0!,#VALUE!,1}".into()),
            CellValue::Number(256.0),
        ],
        false,
    );
}

#[test]
fn normal_density_and_gauss_broadcast_values_and_preserve_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(ROUND(PHI({0,1;-1,#N/A}),8),1)",
            "ARRAYTOTEXT(ROUND(GAUSS({-1,0;1,#DIV/0!}),8),1)",
            "ARRAYTOTEXT(PHI({#N/A,#DIV/0!,TRUE}),1)",
            "ARRAYTOTEXT(ROUND(PHI({\"0\",FALSE}),8),1)",
            "ARRAYTOTEXT(ROUND(GAUSS({\"0\",FALSE,TRUE}),8),1)",
            "TYPE(PHI({0}))",
            "TYPE(GAUSS({0}))",
            "ARRAYTOTEXT(PHI({-1E100,1E100}),1)",
            "ARRAYTOTEXT(GAUSS({-1E100,1E100}),1)",
        ],
        &[
            CellValue::Text("{0.39894228,0.24197072;0.24197072,#N/A}".into()),
            CellValue::Text("{-0.34134475,0;0.34134475,#DIV/0!}".into()),
            CellValue::Text("{#N/A,#DIV/0!,#VALUE!}".into()),
            CellValue::Text("{0.39894228,#VALUE!}".into()),
            CellValue::Text("{0,0,0.34134475}".into()),
            CellValue::Number(64.0),
            CellValue::Number(64.0),
            CellValue::Text("{0,0}".into()),
            CellValue::Text("{-0.5,0.5}".into()),
        ],
        false,
    );
}

#[test]
fn fisher_functions_broadcast_domains_errors_and_singletons() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(ROUND(FISHER({-0.5,0;0.5,1}),8),1)",
            "ARRAYTOTEXT(ROUND(FISHERINV({-1,0;1,#N/A}),8),1)",
            "ARRAYTOTEXT(FISHER({#N/A,#DIV/0!,-1}),1)",
            "ARRAYTOTEXT(FISHERINV({#N/A,#DIV/0!,0}),1)",
            "ARRAYTOTEXT(ROUND(FISHER({FALSE,TRUE,\"0.5\"}),8),1)",
            "ARRAYTOTEXT(ROUND(FISHERINV({FALSE,TRUE,\"0\"}),8),1)",
            "TYPE(FISHER({0}))",
            "TYPE(FISHERINV({0}))",
            "SUM(ABS(FISHERINV(FISHER({-0.9,-0.5,0,0.5,0.9}))-{-0.9,-0.5,0,0.5,0.9}))<1E-12",
        ],
        &[
            CellValue::Text("{-0.54930614,0;0.54930614,#NUM!}".into()),
            CellValue::Text("{-0.76159416,0;0.76159416,#N/A}".into()),
            CellValue::Text("{#N/A,#DIV/0!,#NUM!}".into()),
            CellValue::Text("{#N/A,#DIV/0!,0}".into()),
            CellValue::Text("{0,#NUM!,0.54930614}".into()),
            CellValue::Text("{0,0.76159416,0}".into()),
            CellValue::Number(64.0),
            CellValue::Number(64.0),
            CellValue::Bool(true),
        ],
        false,
    );
}

#[test]
fn standardize_broadcasts_values_means_and_deviations() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(STANDARDIZE({10;20},{2,4},2),1)",
            "ARRAYTOTEXT(STANDARDIZE({4,8},2,{0;2}),1)",
            "ARRAYTOTEXT(STANDARDIZE({4;8},{#N/A,#DIV/0!},2),1)",
            "ARRAYTOTEXT(STANDARDIZE({4,8},2,#N/A),1)",
            "ARRAYTOTEXT(STANDARDIZE({\"4\",TRUE},2,2),1)",
            "TYPE(STANDARDIZE({4},2,2))",
            "STANDARDIZE(42,40,1.5)",
        ],
        &[
            CellValue::Text("{4,3;9,8}".into()),
            CellValue::Text("{#NUM!,#NUM!;1,3}".into()),
            CellValue::Text("{#N/A,#DIV/0!;#N/A,#DIV/0!}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Text("{1,#VALUE!}".into()),
            CellValue::Number(64.0),
            CellValue::Number(2.0 / 1.5),
        ],
        false,
    );
}

#[test]
fn business_day_functions_accept_empty_optional_holiday_arguments() {
    for date1904 in [false, true] {
        assert_formulas(
            &[
                "ARRAYTOTEXT(DAY(WORKDAY(DATE(2024,1,5),{1,2},)),1)",
                "ARRAYTOTEXT(DAY(WORKDAY.INTL(DATE(2024,1,5),{1,2},,)),1)",
                "ARRAYTOTEXT(NETWORKDAYS(DATE(2024,1,5),DATE(2024,1,{8,9}),),1)",
                "ARRAYTOTEXT(NETWORKDAYS.INTL(DATE(2024,1,5),DATE(2024,1,{8,9}),,),1)",
                "DAY(WORKDAY(DATE(2024,1,5),1,Z1))",
                "DAY(WORKDAY.INTL(DATE(2024,1,5),1,1,Z1))",
                "NETWORKDAYS(DATE(2024,1,5),DATE(2024,1,8),Z1)",
                "NETWORKDAYS.INTL(DATE(2024,1,5),DATE(2024,1,8),1,Z1)",
            ],
            &[
                CellValue::Text("{8,9}".into()),
                CellValue::Text("{8,9}".into()),
                CellValue::Text("{2,3}".into()),
                CellValue::Text("{2,3}".into()),
                CellValue::Number(8.0),
                CellValue::Number(8.0),
                CellValue::Number(2.0),
                CellValue::Number(2.0),
            ],
            date1904,
        );
    }
}

#[test]
fn networkdays_broadcasts_endpoints_and_weekends_with_shared_holidays() {
    for date1904 in [false, true] {
        assert_formulas(
            &[
                "ARRAYTOTEXT(NETWORKDAYS(DATE(2024,1,{5;8}),DATE(2024,1,{8,9})),1)",
                "ARRAYTOTEXT(NETWORKDAYS(DATE(2024,1,{5;8}),DATE(2024,1,{8,9}),DATE(2024,1,{8,8})),1)",
                "ARRAYTOTEXT(NETWORKDAYS.INTL(DATE(2024,1,5),DATE(2024,1,{8;9}),{1,11}),1)",
                "ARRAYTOTEXT(NETWORKDAYS.INTL(DATE(2024,1,5),DATE(2024,1,8),{\"0000000\",\"1111111\"}),1)",
                "ARRAYTOTEXT(NETWORKDAYS(DATE(2024,1,{9,8}),DATE(2024,1,5)),1)",
                "ARRAYTOTEXT(NETWORKDAYS(DATE(2024,1,5),{#N/A,1E100}),1)",
                "ARRAYTOTEXT(NETWORKDAYS.INTL(DATE(2024,1,5),DATE(2024,1,{8;9}),{0,#N/A}),1)",
                "ARRAYTOTEXT(NETWORKDAYS(DATE(2024,1,5),DATE(2024,1,{8,9}),{#N/A}),1)",
                "TYPE(NETWORKDAYS(DATE(2024,1,5),DATE(2024,1,{8})))",
                "TYPE(NETWORKDAYS.INTL(DATE(2024,1,5),DATE(2024,1,8),{1}))",
            ],
            &[
                CellValue::Text("{2,3;1,2}".into()),
                CellValue::Text("{1,2;0,1}".into()),
                CellValue::Text("{2,3;3,4}".into()),
                CellValue::Text("{4,0}".into()),
                CellValue::Text("{-3,-2}".into()),
                CellValue::Text("{#N/A,#NUM!}".into()),
                CellValue::Text("{#NUM!,#N/A;#NUM!,#N/A}".into()),
                CellValue::Text("{#N/A,#N/A}".into()),
                CellValue::Number(64.0),
                CellValue::Number(64.0),
            ],
            date1904,
        );
    }
}

#[test]
fn workday_broadcasts_dates_offsets_and_weekends_without_expanding_holidays() {
    for date1904 in [false, true] {
        assert_formulas(
            &[
                "ARRAYTOTEXT(DAY(WORKDAY(DATE(2024,1,{5;8}),{1,2})),1)",
                "ARRAYTOTEXT(DAY(WORKDAY(DATE(2024,1,{5;8}),{1,2},DATE(2024,1,{8,8}))),1)",
                "ARRAYTOTEXT(DAY(WORKDAY.INTL(DATE(2024,1,5),{1;2},{1,11})),1)",
                "ARRAYTOTEXT(DAY(WORKDAY.INTL(DATE(2024,1,5),1,{\"0000011\",\"0000000\"})),1)",
                "ARRAYTOTEXT(WORKDAY(DATE(2024,1,5),{#N/A,1E100}),1)",
                "ARRAYTOTEXT(WORKDAY.INTL(DATE(2024,1,5),{1;2},{0,\"1111111\"}),1)",
                "ARRAYTOTEXT(WORKDAY(DATE(2024,1,5),{1,2},{#N/A}),1)",
                "TYPE(WORKDAY(DATE(2024,1,5),{1}))",
                "TYPE(WORKDAY.INTL(DATE(2024,1,5),1,{1}))",
            ],
            &[
                CellValue::Text("{8,9;9,10}".into()),
                CellValue::Text("{9,10;9,10}".into()),
                CellValue::Text("{8,6;9,8}".into()),
                CellValue::Text("{8,6}".into()),
                CellValue::Text("{#N/A,#NUM!}".into()),
                CellValue::Text("{#NUM!,#VALUE!;#NUM!,#VALUE!}".into()),
                CellValue::Text("{#N/A,#N/A}".into()),
                CellValue::Number(64.0),
                CellValue::Number(64.0),
            ],
            date1904,
        );
    }
}

#[test]
fn workday_rejects_results_outside_the_date_epoch_without_unbounded_walks() {
    for date1904 in [false, true] {
        assert_formulas(
            &[
                "WORKDAY(DATE(9999,12,31),1)",
                "WORKDAY.INTL(DATE(9999,12,31),1)",
                "WORKDAY(DATE(2024,1,5),1E100)",
                "WORKDAY.INTL(DATE(2024,1,5),-1E100)",
                "WORKDAY(DATE(2024,1,5),1.9)-DATE(2024,1,8)",
                "WORKDAY.INTL(DATE(2024,1,5),-1.9)-DATE(2024,1,4)",
                "WORKDAY(DATE(2024,1,5),5,DATE(2024,1,{8,8}))-DATE(2024,1,15)",
                "WORKDAY.INTL(DATE(2024,1,5),1,\"0000000\")-DATE(2024,1,6)",
                "WORKDAY(DATE(2024,1,5),100000)-WORKDAY.INTL(DATE(2024,1,5),100000)",
            ],
            &[
                CellValue::Error("#NUM!".into()),
                CellValue::Error("#NUM!".into()),
                CellValue::Error("#NUM!".into()),
                CellValue::Error("#NUM!".into()),
                CellValue::Number(0.0),
                CellValue::Number(0.0),
                CellValue::Number(0.0),
                CellValue::Number(0.0),
                CellValue::Number(0.0),
            ],
            date1904,
        );
    }
}

#[test]
fn time_components_broadcast_values_errors_and_singleton_shapes() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(HOUR({0,0.5;1.25,-1}),1)",
            "ARRAYTOTEXT(MINUTE({0.125,0.5;0.25,-1}),1)",
            "ARRAYTOTEXT(SECOND({0.125,0.5;0.25,-1}),1)",
            "ARRAYTOTEXT(HOUR({#N/A,#DIV/0!,TRUE,\"0.5\"}),1)",
            "ARRAYTOTEXT(MINUTE({#N/A,#DIV/0!,TRUE,\"0.5\"}),1)",
            "ARRAYTOTEXT(SECOND({#N/A,#DIV/0!,TRUE,\"0.5\"}),1)",
            "TYPE(HOUR({0.5}))",
            "TYPE(MINUTE({0.5}))",
            "TYPE(SECOND({0.5}))",
            "HOUR(TIME(13,14,15))",
            "MINUTE(TIME(13,14,15))",
            "SECOND(TIME(13,14,15))",
        ],
        &[
            CellValue::Text("{0,12;6,#NUM!}".into()),
            CellValue::Text("{0,0;0,#NUM!}".into()),
            CellValue::Text("{0,0;0,#NUM!}".into()),
            CellValue::Text("{#N/A,#DIV/0!,0,12}".into()),
            CellValue::Text("{#N/A,#DIV/0!,0,0}".into()),
            CellValue::Text("{#N/A,#DIV/0!,0,0}".into()),
            CellValue::Number(64.0),
            CellValue::Number(64.0),
            CellValue::Number(64.0),
            CellValue::Number(13.0),
            CellValue::Number(14.0),
            CellValue::Number(15.0),
        ],
        false,
    );
}

#[test]
fn yearfrac_reversed_dates_keep_finite_symmetric_day_counts() {
    for date1904 in [false, true] {
        assert_formulas(
            &[
                "YEARFRAC(DATE(2021,1,1),DATE(2019,1,1),1)",
                "YEARFRAC(DATE(2021,1,1),DATE(2020,1,1),1)",
                "YEARFRAC(DATE(2024,3,31),DATE(2024,2,29),0)",
                "SUM(ABS(YEARFRAC(DATE(2025,6,30),DATE(2020,2,29),{0,1,2,3,4})-YEARFRAC(DATE(2020,2,29),DATE(2025,6,30),{0,1,2,3,4})))<1E-12",
                "SUM(ABS(YEARFRAC(DATE(2021,1,{1;2}),DATE(2020,1,1),{0,1,2,3,4})-YEARFRAC(DATE(2020,1,1),DATE(2021,1,{1;2}),{0,1,2,3,4})))<1E-12",
                "ARRAYTOTEXT(ROUND(YEARFRAC(DATE(2021,1,1),DATE(2019,1,1),{1,5}),4),1)",
            ],
            &[
                CellValue::Number(731.0 / (1096.0 / 3.0)),
                CellValue::Number(1.0),
                CellValue::Number(31.0 / 360.0),
                CellValue::Bool(true),
                CellValue::Bool(true),
                CellValue::Text("{2.0009,#NUM!}".into()),
            ],
            date1904,
        );
    }
}

#[test]
fn rank_functions_broadcast_numbers_and_order_and_preserve_ties() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(RANK.EQ({1;3;7;2},{1,3,3,7},{0,1}),1)",
            "ARRAYTOTEXT(RANK.AVG({1;3;7;2},{1,3,3,7},{0,1}),1)",
            "ARRAYTOTEXT(RANK({3,7},{1,3,3,7}),1)",
            "ARRAYTOTEXT(RANK.EQ({1,2},{#N/A,1}),1)",
            "ARRAYTOTEXT(RANK.AVG(#N/A,{1,2},{0,1}),1)",
            "TYPE(RANK.EQ({3},{1,3,7}))",
            "RANK.AVG(3,{TRUE,\"3\",3})",
        ],
        &[
            CellValue::Text("{4,1;2,2;1,4;#N/A,#N/A}".into()),
            CellValue::Text("{4,1;2.5,2.5;1,4;#N/A,#N/A}".into()),
            CellValue::Text("{2,1}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Number(64.0),
            CellValue::Number(1.0),
        ],
        false,
    );
    assert_formulas(
        &["7", "RANK.EQ(7,A1)", "RANK.AVG(7,A1)"],
        &[
            CellValue::Number(7.0),
            CellValue::Number(1.0),
            CellValue::Number(1.0),
        ],
        false,
    );
}

#[test]
fn percentrank_broadcasts_values_and_precision_without_scale_overflow() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(PERCENTRANK.INC({0,10,20,30},{0;5;15;30},{1,3}),1)",
            "ARRAYTOTEXT(PERCENTRANK.EXC({0,10,20,30},{0;5;15;30}),1)",
            "ARRAYTOTEXT(PERCENTRANK({0,10,20,30},{-1,30}),1)",
            "ARRAYTOTEXT(PERCENTRANK.INC({0,10,20,30},15,{0,3}),1)",
            "ARRAYTOTEXT(PERCENTRANK.EXC({#N/A,1},{0,1}),1)",
            "PERCENTRANK.INC({0,1},0.5,1E100)",
            "PERCENTRANK.INC({-9E307,9E307},0)",
            "PERCENTRANK.EXC({-9E307,9E307},0)",
            "PERCENTRANK.INC({0,10,10,30},10)",
            "TYPE(PERCENTRANK.INC({0,1},{0.5}))",
            "PERCENTRANK.INC({\"text\"},1)",
            "PERCENTRANK.EXC({\"text\"},1)",
        ],
        &[
            CellValue::Text("{0,0;0.1,0.166;0.5,0.5;1,1}".into()),
            CellValue::Text("{0.2;0.3;0.5;0.8}".into()),
            CellValue::Text("{#N/A,1}".into()),
            CellValue::Text("{#NUM!,0.5}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Number(0.5),
            CellValue::Number(0.5),
            CellValue::Number(0.5),
            CellValue::Number(0.333),
            CellValue::Number(64.0),
            CellValue::Error("#NUM!".into()),
            CellValue::Error("#NUM!".into()),
        ],
        false,
    );
}

#[test]
fn percentile_interpolation_avoids_overflow_for_finite_endpoints() {
    assert_formulas(&[
        "PERCENTILE.INC({-9E307,9E307},0)",
        "PERCENTILE.INC({-9E307,9E307},0.5)",
        "PERCENTILE.EXC({-9E307,9E307},1/3)",
        "PERCENTILE.EXC({-9E307,9E307},2/3)",
        "PERCENTILE.EXC({-9E307,9E307},0.5)",
        "QUARTILE.INC({-9E307,9E307},2)",
        "QUARTILE.EXC({-9E307,9E307},2)",
        "PERCENTILE({-9E307,9E307},0.5)",
        "QUARTILE({-9E307,9E307},2)",
        "SUM(ABS(PERCENTILE.INC({-9E307,9E307},{0,0.25,0.5,0.75,1})/9E307-{-1,-0.5,0,0.5,1}))<1E-12",
        "ABS(PERCENTILE.INC({8E307,9E307},0.5)/1E307-8.5)<1E-12",
    ], &[
        CellValue::Number(-9E307),CellValue::Number(0.0),CellValue::Number(-9E307),CellValue::Number(9E307),CellValue::Number(0.0),
        CellValue::Number(0.0),CellValue::Number(0.0),CellValue::Number(0.0),
        CellValue::Number(0.0),CellValue::Bool(true),CellValue::Bool(true),
    ], false);
}

#[test]
fn percentiles_and_quartiles_lift_the_parameter_and_preserve_data_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(PERCENTILE.INC({1,2,3,4},{0,0.25,0.5,1,2,#N/A}),1)",
            "ARRAYTOTEXT(PERCENTILE.EXC({1,2,3,4},{0,0.2,0.5,0.8,1}),1)",
            "ARRAYTOTEXT(QUARTILE.INC({1,2,3,4},{0;1;2;3;4;5}),1)",
            "ARRAYTOTEXT(QUARTILE.EXC({1,2,3,4},{0,1,2,3,4}),1)",
            "ARRAYTOTEXT(PERCENTILE({1,2,3,4},{0,1}),1)",
            "ARRAYTOTEXT(QUARTILE({1,2,3,4},{1,3}),1)",
            "ARRAYTOTEXT(PERCENTILE.INC({#N/A,1},{0,1}),1)",
            "ARRAYTOTEXT(QUARTILE.EXC(#DIV/0!,{1,2}),1)",
            "TYPE(PERCENTILE.INC({1,2},{0.5}))",
            "PERCENTILE.INC({1,2,3,4},0.3)",
        ],
        &[
            CellValue::Text("{1,1.75,2.5,4,#NUM!,#N/A}".into()),
            CellValue::Text("{#NUM!,1,2.5,4,#NUM!}".into()),
            CellValue::Text("{1;1.75;2.5;3.25;4;#NUM!}".into()),
            CellValue::Text("{#NUM!,1.25,2.5,3.75,#NUM!}".into()),
            CellValue::Text("{1,4}".into()),
            CellValue::Text("{1.75,3.25}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Text("{#DIV/0!,#DIV/0!}".into()),
            CellValue::Number(64.0),
            CellValue::Number(1.9),
        ],
        false,
    );
}

#[test]
fn large_and_small_broadcast_ranks_and_preserve_data_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(LARGE({3,1,7,3},{1,2,4,0,5,#N/A}),1)",
            "ARRAYTOTEXT(SMALL({3,1,7,3},{1;2;4}),1)",
            "ARRAYTOTEXT(LARGE({#N/A,3},{1,2}),1)",
            "ARRAYTOTEXT(SMALL(#DIV/0!,{1,2}),1)",
            "ARRAYTOTEXT(LARGE({TRUE,\"9\",5},{1,2}),1)",
            "LARGE({\"text\",TRUE},1)",
            "TYPE(SMALL({2,3},{1}))",
            "ARRAYTOTEXT(LARGE({1,2}, {1,1E100}),1)",
        ],
        &[
            CellValue::Text("{7,3,1,#NUM!,#NUM!,#N/A}".into()),
            CellValue::Text("{1;3;7}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Text("{#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{5,#NUM!}".into()),
            CellValue::Error("#NUM!".into()),
            CellValue::Number(64.0),
            CellValue::Text("{2,#NUM!}".into()),
        ],
        false,
    );
    assert_formulas(
        &["7", "LARGE(A1,1)", "SMALL(A1,1)"],
        &[
            CellValue::Number(7.0),
            CellValue::Number(7.0),
            CellValue::Number(7.0),
        ],
        false,
    );
}

#[test]
fn permutations_broadcast_and_avoid_integer_saturation_and_long_loops() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(PERMUT({3;4},{0,2,5}),1)",
            "ARRAYTOTEXT(PERMUTATIONA({3;4},{0,2,5}),1)",
            "ARRAYTOTEXT(PERMUT(#N/A,{1,2}),1)",
            "ARRAYTOTEXT(PERMUTATIONA({3,4},#DIV/0!),1)",
            "ARRAYTOTEXT(PERMUT({3;4;5},{1;2}),1)",
            "TYPE(PERMUTATIONA({3},2))",
            "PERMUT(100,3)",
            "PERMUT(3.9,2.9)",
            "PERMUT(0,0)",
            "PERMUTATIONA(0,1)",
            "PERMUTATIONA(0,0)",
            "PERMUT(1E100,0)",
            "PERMUT(1E100,1)",
            "PERMUT(1E100,2)",
            "PERMUT(1E100,1E90)",
            "PERMUTATIONA(1,1E100)",
            "PERMUTATIONA(10,1000)",
        ],
        &[
            CellValue::Text("{1,6,#NUM!;1,12,#NUM!}".into()),
            CellValue::Text("{1,9,243;1,16,1024}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Text("{#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{3;12;#N/A}".into()),
            CellValue::Number(64.0),
            CellValue::Number(970200.0),
            CellValue::Number(6.0),
            CellValue::Error("#NUM!".into()),
            CellValue::Error("#NUM!".into()),
            CellValue::Number(1.0),
            CellValue::Number(1.0),
            CellValue::Number(1E100),
            CellValue::Number(1E200),
            CellValue::Error("#NUM!".into()),
            CellValue::Number(1.0),
            CellValue::Error("#NUM!".into()),
        ],
        false,
    );
}

#[test]
fn atan2_broadcasts_coordinates_and_preserves_the_principal_interval() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(DEGREES(ATAN2({1;-1;0},{1,-1,0})),1)",
            "ARRAYTOTEXT(ATAN2(#N/A,{1,2}),1)",
            "ARRAYTOTEXT(ATAN2({1,2},#DIV/0!),1)",
            "ARRAYTOTEXT(DEGREES(ATAN2({1;1;1},{1;-1})),1)",
            "TYPE(ATAN2({1},1))",
            "ARRAYTOTEXT(ATAN2({TRUE,\"1\",\"bad\"},0),1)",
            "ATAN2(-1,-0)=PI()",
            "ABS(DEGREES(ATAN2(1E300,1E300))-45)<1E-12",
        ],
        &[
            CellValue::Text("{45,-45,0;135,-135,180;90,-90,#DIV/0!}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Text("{#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{45;-45;#N/A}".into()),
            CellValue::Number(64.0),
            CellValue::Text("{0,0,#VALUE!}".into()),
            CellValue::Bool(true),
            CellValue::Bool(true),
        ],
        false,
    );
}

#[test]
fn unary_numeric_functions_return_local_errors_for_nonfinite_results() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(EXP({1000,0,#N/A}),1)",
            "ARRAYTOTEXT(ASIN({2,0,-2}),1)",
            "ARRAYTOTEXT(ACOS({2,1,-2}),1)",
            "ARRAYTOTEXT(ACOSH({0,1,-1}),1)",
            "ARRAYTOTEXT(ATANH({-1,0,1}),1)",
            "ARRAYTOTEXT(ACOTH({0.5,-0.5,1,-1}),1)",
            "ARRAYTOTEXT(SINH({1000,0,-1000}),1)",
            "ARRAYTOTEXT(COSH({1000,0,-1000}),1)",
            "TYPE(EXP({0}))",
            "ARRAYTOTEXT(SIN({TRUE,\"0\",\"bad\"}),1)",
        ],
        &[
            CellValue::Text("{#NUM!,1,#N/A}".into()),
            CellValue::Text("{#NUM!,0,#NUM!}".into()),
            CellValue::Text("{#NUM!,0,#NUM!}".into()),
            CellValue::Text("{#NUM!,0,#NUM!}".into()),
            CellValue::Text("{#NUM!,0,#NUM!}".into()),
            CellValue::Text("{#NUM!,#NUM!,#NUM!,#NUM!}".into()),
            CellValue::Text("{#NUM!,0,#NUM!}".into()),
            CellValue::Text("{#NUM!,1,#NUM!}".into()),
            CellValue::Number(64.0),
            CellValue::Text("{0.8414709848078965,0,#VALUE!}".into()),
        ],
        false,
    );
}

#[test]
fn factorials_bound_work_and_preserve_array_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(FACT({0,1.9,5,-1,171,1E100,#N/A}),1)",
            "ARRAYTOTEXT(FACTDOUBLE({0,6,7,-1,301,1E100,#N/A}),1)",
            "FACT(170)>1E300",
            "FACTDOUBLE(300)>1E300",
            "FACT(170.9)=FACT(170)",
            "FACTDOUBLE(300.9)=FACTDOUBLE(300)",
            "TYPE(FACT({5}))",
            "TYPE(FACTDOUBLE({6}))",
            "ARRAYTOTEXT(FACT({TRUE,\"5\",\"bad\"}),1)",
        ],
        &[
            CellValue::Text("{1,1,120,#NUM!,#NUM!,#NUM!,#N/A}".into()),
            CellValue::Text("{1,48,105,#NUM!,#NUM!,#NUM!,#N/A}".into()),
            CellValue::Bool(true),
            CellValue::Bool(true),
            CellValue::Bool(true),
            CellValue::Bool(true),
            CellValue::Number(64.0),
            CellValue::Number(64.0),
            CellValue::Text("{1,120,#VALUE!}".into()),
        ],
        false,
    );
}

#[test]
fn combinations_preserve_exact_integer_results() {
    assert_formulas(
        &[
            "COMBIN(11,5)",
            "COMBIN(11,6)",
            "COMBINA(7,5)",
            "COMBIN(58,23)",
            "COMBINA(36,23)",
            "COMBIN(100,50)",
            "COMBINA(9007199254740992,1)",
            "ARRAYTOTEXT(COMBIN({11;12},{5,6}),1)",
            "MOD(COMBIN(11,5),1)",
        ],
        &[
            CellValue::Number(462.0),
            CellValue::Number(462.0),
            CellValue::Number(462.0),
            CellValue::Number(8799226775309880.0),
            CellValue::Number(8799226775309880.0),
            CellValue::Number(1.008913445455642E29),
            CellValue::Number(9007199254740992.0),
            CellValue::Text("{462,462;792,924}".into()),
            CellValue::Number(0.0),
        ],
        false,
    );
}

#[test]
fn combinations_broadcast_and_bound_large_iteration_counts() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(COMBIN({4;5},{0,2,6}),1)",
            "ARRAYTOTEXT(COMBINA({4;5},{0,2,3}),1)",
            "ARRAYTOTEXT(COMBIN(#N/A,{1,2}),1)",
            "ARRAYTOTEXT(COMBINA({4,5},#DIV/0!),1)",
            "ARRAYTOTEXT(COMBIN({4;5;6},{1;2}),1)",
            "TYPE(COMBINA({4},3))",
            "COMBIN(8.9,2.9)",
            "COMBINA(10,3)",
            "COMBIN(1E100,1E100)",
            "COMBINA(1,1E100)",
            "COMBINA(2,1E100)",
            "COMBIN(1E100,1E90)",
            "COMBINA(1E100,1E90)",
            "COMBIN(2000,1000)",
            "COMBIN(0,0)",
        ],
        &[
            CellValue::Text("{1,6,#NUM!;1,10,#NUM!}".into()),
            CellValue::Text("{1,10,20;1,15,35}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Text("{#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{4;10;#N/A}".into()),
            CellValue::Number(64.0),
            CellValue::Number(28.0),
            CellValue::Number(220.0),
            CellValue::Number(1.0),
            CellValue::Number(1.0),
            CellValue::Number(1E100),
            CellValue::Error("#NUM!".into()),
            CellValue::Error("#NUM!".into()),
            CellValue::Error("#NUM!".into()),
            CellValue::Number(1.0),
        ],
        false,
    );
}

#[test]
fn power_and_log_broadcast_operands_with_local_domain_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(POWER({2;-2;0},{2,3,-1}),1)",
            "ARRAYTOTEXT(POWER({-1,0,1},0.5),1)",
            "ARRAYTOTEXT(LOG({8;64},{2,4}),1)",
            "ARRAYTOTEXT(LOG(8,{0,1,-1,2}),1)",
            "ARRAYTOTEXT(LOG({1,10,100}),1)",
            "ARRAYTOTEXT(POWER(#N/A,{2,3}),1)",
            "ARRAYTOTEXT(LOG({8,64},#DIV/0!),1)",
            "ARRAYTOTEXT(POWER({1;2;3},{2;3}),1)",
            "TYPE(POWER({2},3))",
            "TYPE(LOG(8,{2}))",
            "POWER(1E200,2)",
            "POWER(0,-1)",
            "POWER(0,0)",
        ],
        &[
            CellValue::Text("{4,8,0.5;4,-8,-0.5;0,0,#DIV/0!}".into()),
            CellValue::Text("{#NUM!,0,1}".into()),
            CellValue::Text("{3,1.5;6,3}".into()),
            CellValue::Text("{#NUM!,#DIV/0!,#NUM!,3}".into()),
            CellValue::Text("{0,1,2}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Text("{#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{1;8;#N/A}".into()),
            CellValue::Number(64.0),
            CellValue::Number(64.0),
            CellValue::Error("#NUM!".into()),
            CellValue::Error("#DIV/0!".into()),
            CellValue::Error("#NUM!".into()),
        ],
        false,
    );
}

#[test]
fn modern_multiple_rounding_broadcasts_optional_modes_and_significance() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(CEILING.MATH({5.5;-5.5},2,{0,1}),1)",
            "ARRAYTOTEXT(FLOOR.MATH({5.5;-5.5},2,{0,1}),1)",
            "ARRAYTOTEXT(CEILING.PRECISE({4.3;-4.3},{2,-2,0}),1)",
            "ARRAYTOTEXT(FLOOR.PRECISE({4.3;-4.3},{2,-2,0}),1)",
            "ARRAYTOTEXT(ISO.CEILING({4.3;-4.3},{2,-2,0}),1)",
            "ARRAYTOTEXT(CEILING.MATH({4.3;-4.3}),1)",
            "ARRAYTOTEXT(FLOOR.MATH({4.3;-4.3}),1)",
            "ARRAYTOTEXT(CEILING.MATH({-5.5;-5.5;-5.5},2,{0;1}),1)",
            "TYPE(FLOOR.PRECISE(1,{2}))",
        ],
        &[
            CellValue::Text("{6,6;-4,-6}".into()),
            CellValue::Text("{4,4;-6,-4}".into()),
            CellValue::Text("{6,6,0;-4,-4,0}".into()),
            CellValue::Text("{4,4,0;-6,-6,0}".into()),
            CellValue::Text("{6,6,0;-4,-4,0}".into()),
            CellValue::Text("{5;-4}".into()),
            CellValue::Text("{4;-5}".into()),
            CellValue::Text("{-4;-6;#N/A}".into()),
            CellValue::Number(64.0),
        ],
        false,
    );
    for name in [
        "CEILING.MATH",
        "FLOOR.MATH",
        "CEILING.PRECISE",
        "FLOOR.PRECISE",
        "ISO.CEILING",
    ] {
        let formula = format!("ARRAYTOTEXT({name}(#N/A,{{1,2}}),1)");
        assert_formulas(&[&formula], &[CellValue::Text("{#N/A,#N/A}".into())], false);
    }
}

#[test]
fn floor_and_ceiling_broadcast_significance_and_scalar_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(FLOOR({2.5;-2.5},{2,-2,0}),1)",
            "ARRAYTOTEXT(CEILING({2.5;-2.5},{2,-2,0}),1)",
            "ARRAYTOTEXT(CEILING(#N/A,{1,2}),1)",
            "ARRAYTOTEXT(FLOOR({1,2},#DIV/0!),1)",
            "ARRAYTOTEXT(CEILING({1,2},#DIV/0!),1)",
            "ARRAYTOTEXT(FLOOR({#N/A;3},{1,2}),1)",
            "ARRAYTOTEXT(CEILING({1;2;3},{1;2}),1)",
            "TYPE(FLOOR({1},1))",
            "TYPE(CEILING(1,{1}))",
            "FLOOR(1.58,0.1)",
            "CEILING(0.234,0.01)",
            "FLOOR(0,0)",
        ],
        &[
            CellValue::Text("{2,#NUM!,#DIV/0!;-4,-2,#DIV/0!}".into()),
            CellValue::Text("{4,#NUM!,0;-2,-4,0}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Text("{#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{#N/A,#N/A;3,2}".into()),
            CellValue::Text("{1;2;#N/A}".into()),
            CellValue::Number(64.0),
            CellValue::Number(64.0),
            CellValue::Number(1.5),
            CellValue::Number(0.24),
            CellValue::Number(0.0),
        ],
        false,
    );
}

#[test]
fn mround_broadcasts_multiples_and_keeps_sign_errors_local() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(MROUND({10;-10},{3,-3,0}),1)",
            "ARRAYTOTEXT(MROUND({#N/A;10},{2,3}),1)",
            "ARRAYTOTEXT(MROUND({1,2},#DIV/0!),1)",
            "ARRAYTOTEXT(MROUND({1;2;3},{2;3}),1)",
            "ARRAYTOTEXT(MROUND({TRUE,\"10\",\"bad\"},3),1)",
            "TYPE(MROUND({10},3))",
            "ABS(MROUND(1.3,0.2)-1.4)<1E-12",
            "MROUND(5,-2)",
            "MROUND(0,-3)",
            "MROUND(1E300,1E-300)",
        ],
        &[
            CellValue::Text("{9,#NUM!,0;#NUM!,-9,0}".into()),
            CellValue::Text("{#N/A,#N/A;10,9}".into()),
            CellValue::Text("{#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{2;3;#N/A}".into()),
            CellValue::Text("{#VALUE!,9,#VALUE!}".into()),
            CellValue::Number(64.0),
            CellValue::Bool(true),
            CellValue::Error("#NUM!".into()),
            CellValue::Number(0.0),
            CellValue::Error("#NUM!".into()),
        ],
        false,
    );
}

#[test]
fn mod_and_quotient_broadcast_operands_and_zero_divisor_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(MOD({3;-3},{2,-2,0}),1)",
            "ARRAYTOTEXT(QUOTIENT({10;-10},{3,-3,0}),1)",
            "ARRAYTOTEXT(MOD({#N/A;7},{2,3}),1)",
            "ARRAYTOTEXT(QUOTIENT({1,2},#DIV/0!),1)",
            "ARRAYTOTEXT(MOD({1;2;3},{2;3}),1)",
            "ARRAYTOTEXT(QUOTIENT({TRUE,\"10\",\"bad\"},3),1)",
            "ARRAYTOTEXT(MOD({TRUE,\"10\",\"bad\"},3),1)",
            "TYPE(MOD({3},2))",
            "TYPE(QUOTIENT(3,{2}))",
            "QUOTIENT(4.5,3.1)",
            "QUOTIENT(1E300,1E-300)",
        ],
        &[
            CellValue::Text("{1,-1,#DIV/0!;1,-1,#DIV/0!}".into()),
            CellValue::Text("{3,-3,#DIV/0!;-3,3,#DIV/0!}".into()),
            CellValue::Text("{#N/A,#N/A;1,1}".into()),
            CellValue::Text("{#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{1;2;#N/A}".into()),
            CellValue::Text("{#VALUE!,3,#VALUE!}".into()),
            CellValue::Text("{1,1,#VALUE!}".into()),
            CellValue::Number(64.0),
            CellValue::Number(64.0),
            CellValue::Number(1.0),
            CellValue::Error("#NUM!".into()),
        ],
        false,
    );
}

#[test]
fn trunc_broadcasts_optional_digits_and_handles_coarse_positions() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(TRUNC({8.9;-8.9}),1)",
            "ARRAYTOTEXT(TRUNC({12.345;-12.345},{0,1,2,-1}),1)",
            "ARRAYTOTEXT(TRUNC({#N/A;1.234},{1,2}),1)",
            "ARRAYTOTEXT(TRUNC({1,2},#DIV/0!),1)",
            "ARRAYTOTEXT(TRUNC({1;2;3},{0;1}),1)",
            "TRUNC(1234,-16)",
            "TRUNC(-1234,-1E9)",
            "TRUNC(1.25,1E9)",
            "TRUNC(1E300,2)",
            "ABS(TRUNC(1.2345E-300,302)/1E-300-1.23)<1E-12",
            "TYPE(TRUNC({1}))",
            "ARRAYTOTEXT(TRUNC({TRUE,\"2.15\",\"bad\"},1),1)",
        ],
        &[
            CellValue::Text("{8;-8}".into()),
            CellValue::Text("{12,12.3,12.34,10;-12,-12.3,-12.34,-10}".into()),
            CellValue::Text("{#N/A,#N/A;1.2,1.23}".into()),
            CellValue::Text("{#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{1;2;#N/A}".into()),
            CellValue::Number(0.0),
            CellValue::Number(0.0),
            CellValue::Number(1.25),
            CellValue::Number(1E300),
            CellValue::Bool(true),
            CellValue::Number(64.0),
            CellValue::Text("{1,2.1,#VALUE!}".into()),
        ],
        false,
    );
}

#[test]
fn decimal_rounding_avoids_intermediate_overflow_and_underflow() {
    assert_formulas(
        &[
            "ROUND(1E300,20)",
            "ROUNDUP(1E300,20)",
            "ROUNDDOWN(1E300,20)",
            "ABS(ROUND(1.2345E300,-298)/1E300-1.23)<1E-12",
            "ABS(ROUNDUP(1.2345E300,-298)/1E300-1.24)<1E-12",
            "ABS(ROUNDDOWN(-1.2345E300,-298)/1E300+1.23)<1E-12",
            "ABS(ROUND(1.2345E-300,302)/1E-300-1.23)<1E-12",
            "ABS(ROUNDUP(1.2345E-300,302)/1E-300-1.24)<1E-12",
            "ROUND(1.25,1E9)",
            "ROUND(-1.25,-1E9)",
            "ROUNDUP(1.25,-1E9)",
            "ROUNDDOWN(1.25,-1E9)",
            "ROUNDUP(0,-1E9)",
            "ROUND(9E307,-308)",
            "ROUNDUP(1E308,-309)",
            "ARRAYTOTEXT(ROUND({1.005,-1.475},{2,2}),1)",
        ],
        &[
            CellValue::Number(1E300),
            CellValue::Number(1E300),
            CellValue::Number(1E300),
            CellValue::Bool(true),
            CellValue::Bool(true),
            CellValue::Bool(true),
            CellValue::Bool(true),
            CellValue::Bool(true),
            CellValue::Number(1.25),
            CellValue::Number(0.0),
            CellValue::Error("#NUM!".into()),
            CellValue::Number(0.0),
            CellValue::Number(0.0),
            CellValue::Number(1E308),
            CellValue::Error("#NUM!".into()),
            CellValue::Text("{1.01,-1.48}".into()),
        ],
        false,
    );
}

#[test]
fn rounding_functions_broadcast_values_and_digits_with_local_errors() {
    assert_formulas(&[
        "ARRAYTOTEXT(ROUND({2.15;-1.475},{0,1,2}),1)",
        "ARRAYTOTEXT(ROUNDUP({3.14159;-3.14159},{0,1,3}),1)",
        "ARRAYTOTEXT(ROUNDDOWN({3.14159;-3.14159},{0,1,3}),1)",
        "ARRAYTOTEXT(ROUND({#N/A;1.005},{1,2}),1)",
        "ARRAYTOTEXT(ROUNDUP({1,2},#DIV/0!),1)",
        "ARRAYTOTEXT(ROUNDDOWN({1;2;3},{0;1}),1)",
        "ARRAYTOTEXT(ROUND({1234,-1234},{-1,-2}),1)",
        "TYPE(ROUND({1},0))",
        "ARRAYTOTEXT(ROUND({TRUE,\"2.15\",\"bad\"},1),1)",
        "SUM(ABS(ROUND(YEARFRAC(DATE(2020,1,1),DATE(2021,1,1),{0,1,2,3,4}),4)-{1,1,1.0167,1.0027,1}))<1E-12",
    ], &[
        CellValue::Text("{2,2.2,2.15;-1,-1.5,-1.48}".into()),
        CellValue::Text("{4,3.2,3.142;-4,-3.2,-3.142}".into()),
        CellValue::Text("{3,3.1,3.141;-3,-3.1,-3.141}".into()),
        CellValue::Text("{#N/A,#N/A;1,1.01}".into()),
        CellValue::Text("{#DIV/0!,#DIV/0!}".into()),
        CellValue::Text("{1;2;#N/A}".into()),
        CellValue::Text("{1230,-1200}".into()),
        CellValue::Number(64.0),
        CellValue::Text("{1,2.2,#VALUE!}".into()),
        CellValue::Bool(true),
    ], false);
}

#[test]
fn yearfrac_broadcasts_dates_and_bases_with_element_errors() {
    for epoch in [false, true] {
        assert_formulas(
            &[
                "SUM(ABS(YEARFRAC(DATE(2020,1,{1;2}),DATE(2021,1,1),{0,1,2,3,4})-{360,366,366,366,360;359,365,365,365,359}/{360,366,360,365,360}))<1E-12",
                "ARRAYTOTEXT(YEARFRAC(DATE(2020,1,1),DATE(2021,1,1),{0,5,-1,#N/A}),1)",
                "ARRAYTOTEXT(YEARFRAC({#N/A;#DIV/0!},DATE(2021,1,1),{0,4}),1)",
                "ARRAYTOTEXT(YEARFRAC(DATE(2020,1,{1,2}),DATE(2021,1,1),#N/A),1)",
                "ARRAYTOTEXT(YEARFRAC(DATE(2020,1,{1;2;3}),DATE(2021,1,1),{0;4}),1)",
                "TYPE(YEARFRAC(DATE(2020,1,1),DATE(2021,1,1),{4}))",
                "ROUND(YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),1),8)",
            ],
            &[
                CellValue::Bool(true),
                CellValue::Text("{1,#NUM!,#NUM!,#N/A}".into()),
                CellValue::Text("{#N/A,#N/A;#DIV/0!,#DIV/0!}".into()),
                CellValue::Text("{#N/A,#N/A}".into()),
                CellValue::Text("{1;0.9972222222222222;#N/A}".into()),
                CellValue::Number(64.0),
                CellValue::Number(0.57650273),
            ],
            epoch,
        );
    }
}

#[test]
fn datedif_broadcasts_dates_and_units_and_keeps_element_errors() {
    for epoch in [false, true] {
        assert_formulas(&[
            "ARRAYTOTEXT(DATEDIF(DATE(2020,1,{1;2}),DATE(2021,2,3),{\"Y\",\"M\",\"D\",\"YM\",\"YD\",\"MD\"}),1)",
            "ARRAYTOTEXT(DATEDIF({\"bad\",\"2020-01-01\",#N/A},\"2021-01-01\",\"Y\"),1)",
            "ARRAYTOTEXT(DATEDIF(DATE(2021,1,{1;2}),DATE(2021,1,1),{\"D\",\"Y\"}),1)",
            "ARRAYTOTEXT(DATEDIF(#N/A,DATE(2021,1,1),{\"D\",\"Y\"}),1)",
            "ARRAYTOTEXT(DATEDIF(DATE(2020,1,{1;2;3}),DATE(2021,1,1),{\"Y\";\"M\"}),1)",
            "TYPE(DATEDIF(DATE(2020,1,1),DATE(2021,1,1),{\"Y\"}))",
            "DATEDIF(\"2001-06-01\",\"2002-08-15\",\"YD\")",
        ], &[
            CellValue::Text("{1,13,399,1,33,2;1,13,398,1,32,1}".into()),
            CellValue::Text("{#VALUE!,1,#N/A}".into()),
            CellValue::Text("{0,0;#NUM!,#NUM!}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Text("{1;11;#N/A}".into()),
            CellValue::Number(64.0),
            CellValue::Number(75.0),
        ], epoch);
    }
}

#[test]
fn days360_broadcasts_dates_and_methods_without_collapsing_errors() {
    for epoch in [false, true] {
        assert_formulas(&[
            "ARRAYTOTEXT(DAYS360(DATE(2024,2,{28;29}),DATE(2024,3,31),{FALSE,TRUE}),1)",
            "ARRAYTOTEXT(DAYS360(DATE(2024,2,29),DATE(2024,3,31),{FALSE,TRUE,0,1,\"TRUE\",\"bad\"}),1)",
            "ARRAYTOTEXT(DAYS360({#N/A;#DIV/0!},DATE(2024,3,31),{FALSE,TRUE}),1)",
            "ARRAYTOTEXT(DAYS360(DATE(2024,2,{28,29}),DATE(2024,3,31),#N/A),1)",
            "ARRAYTOTEXT(DAYS360(DATE(2024,1,{1;2;3}),DATE(2024,2,1),{FALSE;TRUE}),1)",
            "TYPE(DAYS360(DATE(2024,1,1),DATE(2024,2,1),{FALSE}))",
            "DAYS360(DATE(2011,1,30),DATE(2011,2,1))",
            "DAYS360(DATE(2011,1,1),DATE(2011,12,31))",
        ], &[
            CellValue::Text("{33,32;30,31}".into()),
            CellValue::Text("{30,31,30,31,31,#VALUE!}".into()),
            CellValue::Text("{#N/A,#N/A;#DIV/0!,#DIV/0!}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Text("{30;29;#N/A}".into()),
            CellValue::Number(64.0),
            CellValue::Number(1.0),
            CellValue::Number(360.0),
        ], epoch);
    }
}

#[test]
fn weekday_broadcasts_numbering_modes_and_preserves_element_errors() {
    for epoch in [false, true] {
        assert_formulas(
            &[
                "ARRAYTOTEXT(WEEKDAY(DATE(2024,1,{1;7}),{1,2,3,11,12,13,14,15,16,17}),1)",
                "ARRAYTOTEXT(WEEKDAY(DATE(2024,1,1),{0,4,21,-1}),1)",
                "ARRAYTOTEXT(WEEKDAY({#N/A;#DIV/0!},{1,2}),1)",
                "ARRAYTOTEXT(WEEKDAY(DATE(2024,1,{1,7}),#N/A),1)",
                "ARRAYTOTEXT(WEEKDAY(DATE(2024,1,{1;7;8}),{1;2}),1)",
                "TYPE(WEEKDAY(DATE(2024,1,1),{2}))",
                "WEEKDAY(DATE(2008,2,14),3)",
            ],
            &[
                CellValue::Text("{2,1,0,1,7,6,5,4,3,2;1,7,6,7,6,5,4,3,2,1}".into()),
                CellValue::Text("{#NUM!,#NUM!,#NUM!,#NUM!}".into()),
                CellValue::Text("{#N/A,#N/A;#DIV/0!,#DIV/0!}".into()),
                CellValue::Text("{#N/A,#N/A}".into()),
                CellValue::Text("{2;7;#N/A}".into()),
                CellValue::Number(64.0),
                CellValue::Number(3.0),
            ],
            epoch,
        );
    }
}

#[test]
fn weeknum_broadcasts_dates_and_return_types_with_local_errors() {
    for epoch in [false, true] {
        assert_formulas(
            &[
                "ARRAYTOTEXT(WEEKNUM(DATE(2021,1,{1;4}),{1,2,21}),1)",
                "ARRAYTOTEXT(WEEKNUM(DATE(2021,1,1),{1,3,0,-1,99,21}),1)",
                "ARRAYTOTEXT(WEEKNUM({#N/A;#DIV/0!},{1,21}),1)",
                "ARRAYTOTEXT(WEEKNUM(DATE(2021,1,{1;4;11}),{1;21}),1)",
                "TYPE(WEEKNUM({1}))",
                "COLUMNS(WEEKNUM(DATE(2021,1,1),{1,21}))",
                "WEEKNUM(DATE(2012,3,9))",
                "WEEKNUM(DATE(2012,3,9),2)",
                "ARRAYTOTEXT(WEEKNUM(DATE(2021,1,{1,4}),#N/A),1)",
            ],
            &[
                CellValue::Text("{1,1,53;2,2,1}".into()),
                CellValue::Text("{1,#NUM!,#NUM!,#NUM!,#NUM!,53}".into()),
                CellValue::Text("{#N/A,#N/A;#DIV/0!,#DIV/0!}".into()),
                CellValue::Text("{1;1;#N/A}".into()),
                CellValue::Number(64.0),
                CellValue::Number(2.0),
                CellValue::Number(10.0),
                CellValue::Number(11.0),
                CellValue::Text("{#N/A,#N/A}".into()),
            ],
            epoch,
        );
    }
}

#[test]
fn timevalue_parses_month_name_date_prefixes_in_arrays() {
    for epoch in [false, true] {
        assert_formulas(
            &[
                "TIMEVALUE(\"22 June 2000 18:45\")",
                "TIMEVALUE(\"22 JUNE 2000 6:45 PM\")",
                "ARRAYTOTEXT(TIMEVALUE({\"bad\",\"22 June 2000 18:45\",#N/A}),1)",
                "TIMEVALUE(\"31 June 2000 18:45\")",
                "TIMEVALUE(\"22 June 2000 25:45\")",
                "TYPE(TIMEVALUE({\"22 June 2000 18:45\"}))",
            ],
            &[
                CellValue::Number(0.78125),
                CellValue::Number(0.78125),
                CellValue::Text("{#VALUE!,0.78125,#N/A}".into()),
                CellValue::Error("#VALUE!".into()),
                CellValue::Error("#VALUE!".into()),
                CellValue::Number(64.0),
            ],
            epoch,
        );
    }
}

#[test]
fn datevalue_rejects_text_dates_outside_the_workbook_epoch() {
    for epoch in [false, true] {
        assert_formulas(
            &[
                "DATEVALUE(\"1899-12-31\")",
                "DATEVALUE(\"0000-01-01\")",
                "DATEVALUE(\"10000-01-01\")",
                "DATEVALUE(\"9999-12-31\")",
                "DATEVALUE(\"1904-01-01\")",
                "DATEVALUE(\"1900-01-01\")",
                "DATEVALUE(\"1900-02-29\")",
                "ARRAYTOTEXT(DATEVALUE({\"1899-12-31\",\"2024-01-01\"}),1)",
                "DAYS(\"1899-12-31\",0)",
            ],
            &[
                CellValue::Error("#VALUE!".into()),
                CellValue::Error("#VALUE!".into()),
                CellValue::Error("#VALUE!".into()),
                CellValue::Number(if epoch { 2957003.0 } else { 2958465.0 }),
                CellValue::Number(if epoch { 0.0 } else { 1462.0 }),
                if epoch {
                    CellValue::Error("#VALUE!".into())
                } else {
                    CellValue::Number(1.0)
                },
                if epoch {
                    CellValue::Error("#VALUE!".into())
                } else {
                    CellValue::Number(60.0)
                },
                CellValue::Text(
                    if epoch {
                        "{#VALUE!,43830}"
                    } else {
                        "{#VALUE!,45292}"
                    }
                    .into(),
                ),
                CellValue::Error("#VALUE!".into()),
            ],
            epoch,
        );
    }
}

#[test]
fn days_preserves_numeric_time_parts_and_broadcasts_text_dates() {
    for epoch in [false, true] {
        assert_formulas(
            &[
                "DAYS(2.75,1.25)",
                "DAYS(1.25,2.75)",
                "DAYS(\"2024-01-02 18:00\",\"2024-01-01 06:00\")",
                "DAYS(DATE(2024,1,2)+0.75,\"2024-01-01 06:00\")",
                "DAYS(\"2024-01-02 18:00\",DATE(2024,1,1)+0.25)",
                "DAYS(\"15-MAR-2021\",\"1-FEB-2021\")",
                "ARRAYTOTEXT(DAYS({3.5;4.5},{1,2}),1)",
                "ARRAYTOTEXT(DAYS(#N/A,{1,2}),1)",
                "ARRAYTOTEXT(DAYS({\"2024-01-02\",\"bad\",#N/A},\"2024-01-01\"),1)",
                "DAYS(\"1\",0)",
                "DAYS(DATE(9999,12,31)+0.5,DATE(9999,12,31))",
                "DAYS(DATE(9999,12,31)+1,0)",
                "TYPE(DAYS({2},1))",
                "COLUMNS(DAYS({2,3},1))",
            ],
            &[
                CellValue::Number(1.5),
                CellValue::Number(-1.5),
                CellValue::Number(1.0),
                CellValue::Number(1.75),
                CellValue::Number(0.75),
                CellValue::Number(42.0),
                CellValue::Text("{2.5,1.5;3.5,2.5}".into()),
                CellValue::Text("{#N/A,#N/A}".into()),
                CellValue::Text("{1,#VALUE!,#N/A}".into()),
                CellValue::Error("#VALUE!".into()),
                CellValue::Number(0.5),
                CellValue::Error("#NUM!".into()),
                CellValue::Number(64.0),
                CellValue::Number(2.0),
            ],
            epoch,
        );
    }
}

#[test]
fn month_shift_functions_broadcast_and_reject_out_of_range_results() {
    for epoch in [false, true] {
        assert_formulas(
            &[
                "ARRAYTOTEXT(TEXT(EDATE(DATE(2024,1,31),{0,1,2,-1}),\"yyyy-mm-dd\"),1)",
                "ARRAYTOTEXT(TEXT(EOMONTH(DATE(2024,1,15),{0,1,-1}),\"yyyy-mm-dd\"),1)",
                "ARRAYTOTEXT(TEXT(EDATE(DATE({2024;2025},1,31),{1,2}),\"yyyy-mm-dd\"),1)",
                "ARRAYTOTEXT(EDATE(#N/A,{0,1}),1)",
                "ARRAYTOTEXT(EOMONTH(#N/A,{0,1}),1)",
                "ARRAYTOTEXT(TEXT(EDATE(DATE(2024,1,31),{\"bad\",1}),\"yyyy-mm-dd\"),1)",
                "TEXT(EDATE(DATE(2024,1,31),-1.9),\"yyyy-mm-dd\")",
                "EDATE(DATE(9999,12,31),1)",
                "EOMONTH(DATE(9999,12,1),0)=DATE(9999,12,31)",
                "EDATE(DATE(2024,1,31),1E10)",
                "EOMONTH(DATE(2024,1,31),-1E10)",
                "EDATE(DATE(2024,1,31),2147483647)",
                "EOMONTH(DATE(2024,1,31),-2147483648)",
                "TYPE(EDATE(DATE({2024},1,31),0))",
                "COLUMNS(EDATE(DATE(2024,1,31),{0,1}))",
            ],
            &[
                CellValue::Text(
                    "{\"2024-01-31\",\"2024-02-29\",\"2024-03-31\",\"2023-12-31\"}".into(),
                ),
                CellValue::Text("{\"2024-01-31\",\"2024-02-29\",\"2023-12-31\"}".into()),
                CellValue::Text(
                    "{\"2024-02-29\",\"2024-03-31\";\"2025-02-28\",\"2025-03-31\"}".into(),
                ),
                CellValue::Text("{#N/A,#N/A}".into()),
                CellValue::Text("{#N/A,#N/A}".into()),
                CellValue::Text("{#VALUE!,\"2024-02-29\"}".into()),
                CellValue::Text("2023-12-31".into()),
                CellValue::Error("#NUM!".into()),
                CellValue::Bool(true),
                CellValue::Error("#NUM!".into()),
                CellValue::Error("#NUM!".into()),
                CellValue::Error("#NUM!".into()),
                CellValue::Error("#NUM!".into()),
                CellValue::Number(64.0),
                CellValue::Number(2.0),
            ],
            epoch,
        );
    }
}

#[test]
fn date_scalar_errors_broadcast_without_collapsing_other_component_shapes() {
    for epoch in [false, true] {
        let shift = if epoch { 1462 } else { 0 };
        assert_formulas(
            &[
                "ARRAYTOTEXT(DATE(#N/A,{1,2},1),1)",
                "ARRAYTOTEXT(DATE(\"bad\",{1,2},1),1)",
                "ARRAYTOTEXT(TEXT(DATE({2024;2025},{1,2},1),\"yyyy-mm-dd\"),1)",
                "ARRAYTOTEXT(DATE({\"bad\",2024},1,1),1)",
                "ARRAYTOTEXT(DATE(10000,{1,2},1),1)",
                "ARRAYTOTEXT(TEXT(DATE({2024;2025},{1;2;3},1),\"yyyy-mm-dd\"),1)",
                "TYPE(DATE({2024},1,1))",
            ],
            &[
                CellValue::Text("{#N/A,#N/A}".into()),
                CellValue::Text("{#VALUE!,#VALUE!}".into()),
                CellValue::Text(
                    "{\"2024-01-01\",\"2024-02-01\";\"2025-01-01\",\"2025-02-01\"}".into(),
                ),
                CellValue::Text(format!("{{#VALUE!,{}}}", 45292 - shift)),
                CellValue::Text("{#NUM!,#NUM!}".into()),
                CellValue::Text("{\"2024-01-01\";\"2025-02-01\";#N/A}".into()),
                CellValue::Number(64.0),
            ],
            epoch,
        );
    }
}

#[test]
fn time_broadcasts_components_and_rejects_components_above_the_excel_limit() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(TEXT(TIME({0;12},{0,30},0),\"hh:mm\"),1)",
            "ARRAYTOTEXT(TEXT(TIME({0;6},{0;30;0},0),\"hh:mm\"),1)",
            "ARRAYTOTEXT(TIME({#N/A,12},0,0),1)",
            "ARRAYTOTEXT(TIME(#N/A,{0,30},0),1)",
            "ARRAYTOTEXT(TEXT(TIME({12,\"bad\",TRUE,32768},0,0),\"hh:mm\"),1)",
            "TYPE(TIME({12},0,0))",
            "COLUMNS(TIME({0,12},0,0))",
            "TIME(32768,0,0)",
            "TIME(0,32768,0)",
            "TIME(0,0,32768)",
            "TEXT(TIME(32767,0,0),\"hh:mm\")",
            "TEXT(TIME(0,32767,0),\"hh:mm\")",
            "TEXT(TIME(0,0,32767),\"hh:mm:ss\")",
        ],
        &[
            CellValue::Text("{\"00:00\",\"00:30\";\"12:00\",\"12:30\"}".into()),
            CellValue::Text("{\"00:00\";\"06:30\";#N/A}".into()),
            CellValue::Text("{#N/A,0.5}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Text("{\"12:00\",#VALUE!,\"01:00\",#NUM!}".into()),
            CellValue::Number(64.0),
            CellValue::Number(2.0),
            CellValue::Error("#NUM!".into()),
            CellValue::Error("#NUM!".into()),
            CellValue::Error("#NUM!".into()),
            CellValue::Text("07:00".into()),
            CellValue::Text("18:07".into()),
            CellValue::Text("09:06:07".into()),
        ],
        false,
    );
}

#[test]
fn datevalue_and_timevalue_convert_arrays_with_element_errors_and_shapes() {
    for epoch in [false, true] {
        let shift = if epoch { 1462 } else { 0 };
        assert_formulas(
            &[
                "ARRAYTOTEXT(DATEVALUE({\"2024-01-02\",\"2024-02-29\"}),1)",
                "ARRAYTOTEXT(DATEVALUE({\"bad\",\"2024-01-02\",#N/A}),1)",
                "ARRAYTOTEXT(TIMEVALUE({\"12:00\",\"06:00\",\"bad\",#N/A}),1)",
                "ARRAYTOTEXT(TIMEVALUE({\"bad\",\"12:00\",#N/A}),1)",
                "TYPE(DATEVALUE({\"2024-01-02\"}))",
                "TYPE(TIMEVALUE({\"12:00\"}))",
                "COLUMNS(DATEVALUE({\"2024-01-02\",\"2024-02-29\"}))",
                "COLUMNS(TIMEVALUE({\"12:00\",\"06:00\"}))",
                "DATEVALUE(1)",
                "DATEVALUE(TRUE)",
                "DATEVALUE(Z1)",
            ],
            &[
                CellValue::Text(format!("{{{},{}}}", 45293 - shift, 45351 - shift)),
                CellValue::Text(format!("{{#VALUE!,{},#N/A}}", 45293 - shift)),
                CellValue::Text("{0.5,0.25,#VALUE!,#N/A}".into()),
                CellValue::Text("{#VALUE!,0.5,#N/A}".into()),
                CellValue::Number(64.0),
                CellValue::Number(64.0),
                CellValue::Number(2.0),
                CellValue::Number(2.0),
                CellValue::Error("#VALUE!".into()),
                CellValue::Error("#VALUE!".into()),
                CellValue::Error("#VALUE!".into()),
            ],
            epoch,
        );
    }
}

#[test]
fn text_leading_decimal_context_looks_through_literals_but_stops_at_dates_and_sections() {
    for epoch in [false, true] {
        assert_formulas(
            &[
                "TEXT(1.23,\".\"\"x\"\"00\")",
                "TEXT(-12.34,\".\"\"x\"\"00\")",
                "TEXT(1.23,\".\\x00\")",
                "TEXT(1.23,\".\\;00\")",
                "TEXT(1.23,\".\"\"x\"\" 00\")",
                "TEXT(1,\".\"\"x\"\"??\")",
                "TEXT(1.23,\".\"\"x\"\";0.00\")",
                "TEXT(1.23,\".\"\"00\"\"\")",
                "TEXT(DATE(2024,1,2),\".\"\"x\"\"yyyy-mm-dd\")",
                "TEXT(0.5,\".\"\"x\"\"hh:mm:ss.000\")",
                "ARRAYTOTEXT(TEXT({1.23,-12.34},\".\"\"x\"\"00\"),1)",
            ],
            &[
                CellValue::Text("1.x23".into()),
                CellValue::Text("-12.x34".into()),
                CellValue::Text("1.x23".into()),
                CellValue::Text("1.;23".into()),
                CellValue::Text("1.x 23".into()),
                CellValue::Text("1.x  ".into()),
                CellValue::Text(".x".into()),
                CellValue::Text(".00".into()),
                CellValue::Text(".x2024-01-02".into()),
                CellValue::Text(".x12:00:00.000".into()),
                CellValue::Text("{\"1.x23\",\"-12.x34\"}".into()),
            ],
            epoch,
        );
    }
}

#[test]
fn text_keeps_decimal_points_at_their_format_token_positions() {
    assert_formulas(
        &[
            "TEXT(1.25,\"0.\")",
            "TEXT(1.5,\"0.\")",
            "TEXT(-1.5,\"0.\")",
            "TEXT(1.23,\"0.\"\"x\"\"00\")",
            "TEXT(1.23,\"0.\\x00\")",
            "TEXT(0.0123,\"0.\"\"pct\"\"00%\")",
            "TEXT(12.34,\".00\")",
            "TEXT(-12.34,\".00\")",
            "TEXT(12,\"0\"\".\"\"00\")",
            "TEXT(1,\"0.??\")",
            "ARRAYTOTEXT(TEXT({1.25,2.5},\"0.\"),1)",
        ],
        &[
            CellValue::Text("1.".into()),
            CellValue::Text("2.".into()),
            CellValue::Text("-2.".into()),
            CellValue::Text("1.x23".into()),
            CellValue::Text("1.x23".into()),
            CellValue::Text("1.pct23%".into()),
            CellValue::Text("12.34".into()),
            CellValue::Text("-12.34".into()),
            CellValue::Text("0.12".into()),
            CellValue::Text("1.  ".into()),
            CellValue::Text("{\"1.\",\"3.\"}".into()),
        ],
        false,
    );
}

#[test]
fn text_grouping_ignores_suppressed_leading_placeholders() {
    assert_formulas(
        &[
            "TEXT(1234.5,\"###,##0.00\")",
            "TEXT(1234567.89,\"#########,##0.00\")",
            "TEXT(12,\"###,##0.00\")",
            "TEXT(0.2,\"###,##0.00\")",
            "TEXT(-1234.5,\"###,##0.00\")",
            "TEXT(12,\"000,000\")",
            "TEXT(1234567,\"###,##0,.00\")",
            "TEXT(1234567,\"###,##0,,.00\")",
            "ARRAYTOTEXT(TEXT({1234.5,12,0.2},\"###,##0.00\"),1)",
        ],
        &[
            CellValue::Text("1,234.50".into()),
            CellValue::Text("1,234,567.89".into()),
            CellValue::Text("12.00".into()),
            CellValue::Text("0.20".into()),
            CellValue::Text("-1,234.50".into()),
            CellValue::Text("000,012".into()),
            CellValue::Text("1,234.57".into()),
            CellValue::Text("1.23".into()),
            CellValue::Text("{\"1,234.50\",\"12.00\",\"0.20\"}".into()),
        ],
        false,
    );
}

#[test]
fn text_rounds_decimal_half_values_away_from_zero_across_numeric_formats() {
    assert_formulas(
        &[
            "TEXT(1.25,\"0.0\")",
            "TEXT(-1.25,\"0.0\")",
            "TEXT(2.15,\"0.0\")",
            "TEXT(2.675,\"0.00\")",
            "TEXT(-1.475,\"0.00\")",
            "TEXT(9.95,\"0.0\")",
            "TEXT(999.995,\"0.00\")",
            "TEXT(0.9996,\"0.000\")",
            "TEXT(9.9996,\"0.000\")",
            "TEXT(12345,\"0.###E+00\")",
            "TEXT(1.25,\"##0.0E+0\")",
            "TEXT(999.95,\"##0.0E+0\")",
            "TEXT(1.25,\"0.0\")=FIXED(1.25,1,TRUE)",
            "TEXT(1E308,\"0.00\")",
            "ARRAYTOTEXT(TEXT({-1.25,1.25,9.95},\"0.0\"),1)",
        ],
        &[
            CellValue::Text("1.3".into()),
            CellValue::Text("-1.3".into()),
            CellValue::Text("2.2".into()),
            CellValue::Text("2.68".into()),
            CellValue::Text("-1.48".into()),
            CellValue::Text("10.0".into()),
            CellValue::Text("1000.00".into()),
            CellValue::Text("1.000".into()),
            CellValue::Text("10.000".into()),
            CellValue::Text("1.235E+04".into()),
            CellValue::Text("1.3E+0".into()),
            CellValue::Text("1.0E+3".into()),
            CellValue::Bool(true),
            CellValue::Text(format!("1{}.00", "0".repeat(308))),
            CellValue::Text("{\"-1.3\",\"1.3\",\"10.0\"}".into()),
        ],
        false,
    );
}

#[test]
fn text_engineering_formats_group_the_exponent_by_integer_placeholder_count() {
    assert_formulas(
        &[
            "TEXT(12200000,\"#0.0E+0\")",
            "TEXT(12200000,\"##0.0E+0\")",
            "TEXT(123456,\"##0.0E+0\")",
            "TEXT(123456,\"#0.0E+0\")",
            "TEXT(0.0123,\"##0.0E+0\")",
            "TEXT(0.000123,\"##0.0E+0\")",
            "TEXT(0.00123,\"#0.0E+0\")",
            "TEXT(999.96,\"##0.0E+0\")",
            "TEXT(9.99,\"##0.0E+0\")",
            "TEXT(0,\"##0.0E+0\")",
            "TEXT(-123456,\"##0.0E+0\")",
            "TEXT(9.999E307,\"##0.0E+0\")",
            "ARRAYTOTEXT(TEXT({123456,0.0123,999.96},\"##0.0E+0\"),1)",
        ],
        &[
            CellValue::Text("12.2E+6".into()),
            CellValue::Text("12.2E+6".into()),
            CellValue::Text("123.5E+3".into()),
            CellValue::Text("12.3E+4".into()),
            CellValue::Text("12.3E-3".into()),
            CellValue::Text("123.0E-6".into()),
            CellValue::Text("12.3E-4".into()),
            CellValue::Text("1.0E+3".into()),
            CellValue::Text("10.0E+0".into()),
            CellValue::Text("0.0E+0".into()),
            CellValue::Text("-123.5E+3".into()),
            CellValue::Text("100.0E+306".into()),
            CellValue::Text("{\"123.5E+3\",\"12.3E-3\",\"1.0E+3\"}".into()),
        ],
        false,
    );
}

#[test]
fn text_scientific_formats_normalize_carries_before_emitting_the_exponent() {
    assert_formulas(
        &[
            "TEXT(999.9,\"0.00E+00\")",
            "TEXT(9.999,\"0.00E+00\")",
            "TEXT(0.009999,\"0.00E+00\")",
            "TEXT(-999.9,\"0.00E+00\")",
            "TEXT(0,\"0.00E+00\")",
            "TEXT(1,\"0.00E+00\")",
            "TEXT(9.12,\"0.00E+00\")",
            "TEXT(999.9,\"0.00E-00\")",
            "TEXT(9.999E307,\"0.00E+00\")",
            "TEXT(1E-300,\"0.00E+00\")",
            "ARRAYTOTEXT(TEXT({999.9,9.999,0.009999},\"0.00E+00\"),1)",
        ],
        &[
            CellValue::Text("1.00E+03".into()),
            CellValue::Text("1.00E+01".into()),
            CellValue::Text("1.00E-02".into()),
            CellValue::Text("-1.00E+03".into()),
            CellValue::Text("0.00E+00".into()),
            CellValue::Text("1.00E+00".into()),
            CellValue::Text("9.12E+00".into()),
            CellValue::Text("1.00E03".into()),
            CellValue::Text("1.00E+308".into()),
            CellValue::Text("1.00E-300".into()),
            CellValue::Text("{\"1.00E+03\",\"1.00E+01\",\"1.00E-02\"}".into()),
        ],
        false,
    );
}

#[test]
fn text_optional_decimal_placeholders_keep_spacing_and_decimal_separator() {
    assert_formulas(
        &[
            "TEXT(1,\"0.##\")",
            "TEXT(1.2,\"0.##\")",
            "TEXT(1.25,\"0.##\")",
            "TEXT(1,\"0.??\")",
            "TEXT(1.2,\"0.??\")",
            "TEXT(1.25,\"0.??\")",
            "TEXT(1,\"0.0?\")",
            "TEXT(1.2,\"0.0?\")",
            "TEXT(1,\"0.??%\")",
            "TEXT(-1.2,\"0.??\")",
            "ARRAYTOTEXT(TEXT({1,1.2,1.25},\"0.??\"),1)",
        ],
        &[
            CellValue::Text("1.".into()),
            CellValue::Text("1.2".into()),
            CellValue::Text("1.25".into()),
            CellValue::Text("1.  ".into()),
            CellValue::Text("1.2 ".into()),
            CellValue::Text("1.25".into()),
            CellValue::Text("1.0 ".into()),
            CellValue::Text("1.2 ".into()),
            CellValue::Text("100.  %".into()),
            CellValue::Text("-1.2 ".into()),
            CellValue::Text("{\"1.  \",\"1.2 \",\"1.25\"}".into()),
        ],
        false,
    );
}

#[test]
fn text_numeric_formats_keep_negative_signs_at_precision_boundaries() {
    assert_formulas(
        &[
            "TEXT(-1,\"0\")",
            "TEXT(-0.01,\"0.00\")",
            "TEXT(-0.006,\"0.00\")",
            "TEXT(-0.1,\"0.0\")",
            "TEXT(-0.01,\".00\")",
            "TEXT(0.01,\".00\")",
            "TEXT(-12.34,\".00\")",
            "TEXT(12.34,\".00\")",
            "TEXT(-0.01,\"0%\")",
            "TEXT(-0.0001,\"0.00%\")",
            "TEXT(-0.01,\"[>=0]0.00;[<0]0.00;0\")",
            "TEXT(-0.01,\"0.00;(0.00)\")",
            "TEXT(0,\"0.00\")",
            "ARRAYTOTEXT(TEXT({-1,-0.01,0},\"0.00\"),1)",
        ],
        &[
            CellValue::Text("-1".into()),
            CellValue::Text("-0.01".into()),
            CellValue::Text("-0.01".into()),
            CellValue::Text("-0.1".into()),
            CellValue::Text("-.01".into()),
            CellValue::Text(".01".into()),
            CellValue::Text("-12.34".into()),
            CellValue::Text("12.34".into()),
            CellValue::Text("-1%".into()),
            CellValue::Text("-0.01%".into()),
            CellValue::Text("-0.01".into()),
            CellValue::Text("(0.01)".into()),
            CellValue::Text("0.00".into()),
            CellValue::Text("{\"-1.00\",\"-0.01\",\"0.00\"}".into()),
        ],
        false,
    );
}

#[test]
fn text_preserves_empty_format_sections_including_trailing_sections() {
    assert_formulas(
        &[
            "TEXT(0,\"0;;\")",
            "TEXT(-2,\"0;\")",
            "TEXT(2,\";0\")",
            "TEXT(-2,\";0\")",
            "TEXT(2,\";;;\")",
            "TEXT(-2,\"General;\")",
            "TEXT(2,\"\")",
            "TEXT(50,\"[>100]0.0;\")",
            "TEXT(0,\"[>100]0.0;[<0]0.00;\")",
            "TEXT(0.5,\"[>=1]0/0;\")",
            "TEXT(2,\"0;;;;\")",
            "ARRAYTOTEXT(TEXT({2,-2,0},\"0;;\"),1)",
        ],
        &[
            CellValue::Text("".into()),
            CellValue::Text("".into()),
            CellValue::Text("".into()),
            CellValue::Text("2".into()),
            CellValue::Text("".into()),
            CellValue::Text("".into()),
            CellValue::Text("".into()),
            CellValue::Text("".into()),
            CellValue::Text("".into()),
            CellValue::Text("".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Text("{\"2\",\"\",\"\"}".into()),
        ],
        false,
    );
}

#[test]
fn text_fraction_sections_share_conditions_with_decimal_sections() {
    assert_formulas(
        &[
            "TEXT(1.5,\"[>=1]0/0;0.00\")",
            "TEXT(0.5,\"[>=1]0/0;0.00\")",
            "TEXT(-1.5,\"[>=0]0.00;[<0]0/0;0\")",
            "TEXT(2.25,\"[>=2]# ?/8;0.00\")",
            "TEXT(-1.5,\"0/0;0/0\")",
            "TEXT(-1.5,\"0/0\")",
            "TEXT(2,\"0\\;;0/0\")",
            "TEXT(-1.5,\"0\\;;0/0\")",
            "TEXT(0.5,\"[>=1]0/0;hh:mm\")",
            "TEXT(1.5,\"[<1]0/0;General\")",
            "TEXT(2,\"[<1]hh:mm;0/0\")",
            "ARRAYTOTEXT(TEXT({1.5,0.5,-1.5},\"[>=1]0/0;0.00\"),1)",
        ],
        &[
            CellValue::Text("3/2".into()),
            CellValue::Text("0.50".into()),
            CellValue::Text("-3/2".into()),
            CellValue::Text("2 2/8".into()),
            CellValue::Text("3/2".into()),
            CellValue::Text("-3/2".into()),
            CellValue::Text("2;".into()),
            CellValue::Text("3/2".into()),
            CellValue::Text("12:00".into()),
            CellValue::Text("1.5".into()),
            CellValue::Text("2/1".into()),
            CellValue::Text("{\"3/2\",\"0.50\",\"-1.50\"}".into()),
        ],
        false,
    );
}

#[test]
fn text_conditions_preserve_date_time_and_general_section_metadata() {
    for epoch in [false, true] {
        assert_formulas(
            &[
                "TEXT(0.5,\"[<1]hh:mm;0.00\")",
                "TEXT(2,\"[<1]hh:mm;0.00\")",
                "TEXT(0.5,\"[>=1]General;0.00\")",
                "TEXT(2,\"[>=1]General;0.00\")",
                "TEXT(DATE(2024,1,2),\"[>=1]yyyy-mm-dd;0.00\")",
                "TEXT(0.5,\"[<>0.5]General;hh:mm\")",
                "ARRAYTOTEXT(TEXT({0.5,2},\"[<1]hh:mm;0.00\"),1)",
            ],
            &[
                CellValue::Text("12:00".into()),
                CellValue::Text("2.00".into()),
                CellValue::Text("0.50".into()),
                CellValue::Text("2".into()),
                CellValue::Text("2024-01-02".into()),
                CellValue::Text("12:00".into()),
                CellValue::Text("{\"12:00\",\"2.00\"}".into()),
            ],
            epoch,
        );
    }
}

#[test]
fn text_not_equal_conditions_select_matching_and_fallback_sections() {
    assert_formulas(
        &[
            "TEXT(2,\"[<>0]0.00;0.0\")",
            "TEXT(-2,\"[<>0]0.00;0.0\")",
            "TEXT(0,\"[<>0]0.00;0.0\")",
            "TEXT(-150,\"[<>-1.5E2]0.00;0.0\")",
            "TEXT(-149,\"[<>-1.5E2]0.00;0.0\")",
            "TEXT(0,\"[<>0]0.00;[=0]0.000;0\")",
            "ARRAYTOTEXT(TEXT({2,-2,0},\"[<>0]0.00;0.0\"),1)",
        ],
        &[
            CellValue::Text("2.00".into()),
            CellValue::Text("-2.00".into()),
            CellValue::Text("0.0".into()),
            CellValue::Text("-150.0".into()),
            CellValue::Text("-149.00".into()),
            CellValue::Text("0.000".into()),
            CellValue::Text("{\"2.00\",\"-2.00\",\"0.0\"}".into()),
        ],
        false,
    );
}

#[test]
fn text_number_format_conditions_override_sign_sections() {
    assert_formulas(
        &[
            "TEXT(101,\"[>100]0.0;[<=-100]0.00;0.000\")",
            "TEXT(100,\"[>100]0.0;[<=-100]0.00;0.000\")",
            "TEXT(-100,\"[>100]0.0;[<=-100]0.00;0.000\")",
            "TEXT(-99,\"[>100]0.0;[<=-100]0.00;0.000\")",
            "TEXT(0,\"[>100]0.0;[<=-100]0.00;0.000\")",
            "TEXT(2,\"[=2]0.00;0.0\")",
            "TEXT(3,\"[=2]0.00;0.0\")",
            "TEXT(10,\"[>=10]0.00;0.0\")",
            "TEXT(9,\"[<10]0.00;0.0\")",
            "TEXT(-2,\"0.0;(0.0);0\")",
            "ARRAYTOTEXT(TEXT({101,100,-100,-99,0},\"[>100]0.0;[<=-100]0.00;0.000\"),1)",
        ],
        &[
            CellValue::Text("101.0".into()),
            CellValue::Text("100.000".into()),
            CellValue::Text("-100.00".into()),
            CellValue::Text("-99.000".into()),
            CellValue::Text("0.000".into()),
            CellValue::Text("2.00".into()),
            CellValue::Text("3.0".into()),
            CellValue::Text("10.00".into()),
            CellValue::Text("9.00".into()),
            CellValue::Text("(2.0)".into()),
            CellValue::Text("{\"101.0\",\"100.000\",\"-100.00\",\"-99.000\",\"0.000\"}".into()),
        ],
        false,
    );
}

#[test]
fn general_scientific_format_normalizes_exponent_carries() {
    assert_formulas(
        &[
            "TEXT(999999999999,\"General\")",
            "TEXT(-999999999999,\"General\")",
            "TEXT(9.999999E307,\"General\")",
            "TEXT(1.23456789E20,\"General\")",
            "TEXT(1E-9,\"General\")",
            "TEXT(0,\"General\")",
        ],
        &[
            CellValue::Text("1E+12".into()),
            CellValue::Text("-1E+12".into()),
            CellValue::Text("1E+308".into()),
            CellValue::Text("1.23457E+20".into()),
            CellValue::Text("1E-09".into()),
            CellValue::Text("0".into()),
        ],
        false,
    );
}

#[test]
fn text_ampm_tokens_keep_requested_case_and_short_period_names() {
    assert_formulas(
        &[
            "TEXT(0,\"h:mm AM/PM\")",
            "TEXT(0.5,\"h:mm am/pm\")",
            "TEXT(0,\"h:mm A/P\")",
            "TEXT(0.75,\"h:mm a/p\")",
            "TEXT(0.5-0.0001/86400,\"h:mm:ss.000 AM/PM\")",
            "TEXT(1-0.0001/86400,\"h:mm:ss.000 a/p\")",
            "ARRAYTOTEXT(TEXT({0,0.5,0.75},\"h:mm a/p\"),1)",
        ],
        &[
            CellValue::Text("12:00 AM".into()),
            CellValue::Text("12:00 pm".into()),
            CellValue::Text("12:00 A".into()),
            CellValue::Text("6:00 p".into()),
            CellValue::Text("12:00:00.000 PM".into()),
            CellValue::Text("12:00:00.000 a".into()),
            CellValue::Text("{\"12:00 a\",\"12:00 p\",\"6:00 p\"}".into()),
        ],
        false,
    );
}

#[test]
fn text_fractional_seconds_round_once_and_carry_into_the_workbook_date() {
    for epoch in [false, true] {
        assert_formulas(
            &[
                "TEXT(59.9994/86400,\"hh:mm:ss.000\")",
                "TEXT(59.9996/86400,\"hh:mm:ss.000\")",
                "TEXT(1.25/86400,\"ss.0\")",
                "TEXT(1.234/86400,\"ss.00\")",
                "TEXT(90061.234/86400,\"[h]:mm:ss.000\")",
                "TEXT(90061.234/86400,\"[s].000\")",
                "TEXT(DATE(2024,1,1)+86399.9996/86400,\"yyyy-mm-dd hh:mm:ss.000\")",
                "TEXT(3735.8/86400,\"mm:ss.00\")",
                "TEXT(3735.8/86400,\"[ss].00\")",
                "TEXT(DATE(2024,1,1)+0.5,\"hh:mm yyyy-mm-dd\")",
            ],
            &[
                CellValue::Text("00:00:59.999".into()),
                CellValue::Text("00:01:00.000".into()),
                CellValue::Text("01.3".into()),
                CellValue::Text("01.23".into()),
                CellValue::Text("25:01:01.234".into()),
                CellValue::Text("90061.234".into()),
                CellValue::Text("2024-01-02 00:00:00.000".into()),
                CellValue::Text("02:15.80".into()),
                CellValue::Text("3735.80".into()),
                CellValue::Text("12:00 2024-01-01".into()),
            ],
            epoch,
        );
    }
}

#[test]
fn text_slices_coerce_numeric_text_and_boolean_positions_per_element() {
    assert_formulas(
        &[
            "LEFT(\"abcd\",\"2\")",
            "RIGHT(\"abcd\",TRUE)",
            "MID(\"abcd\",TRUE,\"2\")",
            "ARRAYTOTEXT(LEFT(\"abcd\",{\"2\",TRUE,FALSE,\"bad\",#N/A}),1)",
            "ARRAYTOTEXT(RIGHT(\"abcd\",{\"2\",TRUE,FALSE,\"bad\",#N/A}),1)",
            "ARRAYTOTEXT(MID(\"abcd\",{\"2\",TRUE,FALSE,\"bad\",#N/A},\"2\"),1)",
            "LEFT(\"abc\",\"NaN\")",
            "RIGHT(\"abc\",\"inf\")",
        ],
        &[
            CellValue::Text("ab".into()),
            CellValue::Text("d".into()),
            CellValue::Text("ab".into()),
            CellValue::Text("{\"ab\",\"a\",\"\",#VALUE!,#N/A}".into()),
            CellValue::Text("{\"cd\",\"d\",\"\",#VALUE!,#N/A}".into()),
            CellValue::Text("{\"bc\",\"ab\",#VALUE!,#VALUE!,#N/A}".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
        ],
        false,
    );
}

#[test]
fn code_maps_arrays_preserving_windows_code_page_errors_and_shape() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(CODE({\"A\",\"€\",\"ÿ\",12,TRUE,\"\",#N/A}),1)",
            "CODE({\"\"})",
            "ROWS(A2#)",
            "SUM(CODE({\"A\",\"B\"}))",
            "ARRAYTOTEXT(CODE(CHAR({65,128,255})),1)",
        ],
        &[
            CellValue::Text("{65,128,255,49,84,#VALUE!,#N/A}".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(1.0),
            CellValue::Number(131.0),
            CellValue::Text("{65,128,255}".into()),
        ],
        false,
    );
}

#[test]
fn japanese_width_functions_map_arrays_with_voiced_kana_and_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(ASC({\"ＡＢＣ　ガ\",\"日本😀\",TRUE,12,#N/A}),1)",
            "ARRAYTOTEXT(DBCS({\"ABC ｶﾞ\",\"ｳﾞﾊﾟ\",\"日本😀\",TRUE,12,#N/A}),1)",
            "SUM(--EXACT(DBCS({\"abc\",\"ｶﾞ\"}),JIS({\"abc\",\"ｶﾞ\"})))",
            "ASC({#N/A})",
            "ROWS(A4#)",
            "DBCS({#N/A})",
            "ROWS(A6#)",
        ],
        &[
            CellValue::Text("{\"ABC ｶﾞ\",\"日本😀\",\"TRUE\",\"12\",#N/A}".into()),
            CellValue::Text(
                "{\"ＡＢＣ　ガ\",\"ヴパ\",\"日本😀\",\"ＴＲＵＥ\",\"１２\",#N/A}".into(),
            ),
            CellValue::Number(2.0),
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
        ],
        false,
    );
}

#[test]
fn width_conversion_checks_expanded_utf16_length() {
    assert_formulas(
        &[
            "LEN(ASC(REPT(\"ガ\",16383)))",
            "ASC(REPT(\"ガ\",16384))",
            "LEN(DBCS(REPT(\"ｶﾞ\",16383)))",
            "LEN(ASC(REPT(\"Ａ\",32767)))",
        ],
        &[
            CellValue::Number(32766.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(16383.0),
            CellValue::Number(32767.0),
        ],
        false,
    );
}

#[test]
fn concatenate_broadcasts_arrays_without_flattening_like_concat() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(CONCATENATE({\"a\",\"b\"},{1;2}),1)",
            "ARRAYTOTEXT(CONCATENATE({12,TRUE,\"\",#N/A},\"!\"),1)",
            "ARRAYTOTEXT(CONCATENATE({\"a\",\"b\"},NA()),1)",
            "CONCATENATE({#N/A},\"x\")",
            "ROWS(A4#)",
        ],
        &[
            CellValue::Text("{\"a1\",\"b1\";\"a2\",\"b2\"}".into()),
            CellValue::Text("{\"12!\",\"TRUE!\",\"!\",#N/A}".into()),
            CellValue::Text("{#N/A,#N/A}".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
        ],
        false,
    );
    let formula = format!("CONCATENATE({})", vec!["\"a\""; 255].join(","));
    assert_formulas(&[&formula], &[CellValue::Text("a".repeat(255))], false);
}

#[test]
fn concatenate_does_not_return_text_beyond_the_cell_limit() {
    assert_formulas(
        &[
            "LEN(CONCATENATE(REPT(\"日\",32766),\"本\"))",
            "CONCATENATE(REPT(\"日\",32767),\"本\")",
            "CONCATENATE(REPT(\"😀\",16383),\"😀\")",
        ],
        &[
            CellValue::Number(32767.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
        ],
        false,
    );
}

#[test]
fn concat_flattens_arrays_and_enforces_utf16_output_limits() {
    assert_formulas(
        &[
            "CONCAT({\"a\",12;TRUE,\"b\"},\"!\")",
            "CONCAT({\"a\",#N/A})",
            "LEN(CONCAT(REPT(\"日\",32766),\"本\"))",
            "CONCAT(REPT(\"日\",32767),\"本\")",
            "LEN(CONCAT(REPT(\"😀\",16383),\"a\"))",
            "CONCAT(REPT(\"😀\",16383),\"😀\")",
            "CONCAT(Z1:XFD1048576)",
        ],
        &[
            CellValue::Text("a12TRUEb!".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Number(32767.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(16384.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Text("".into()),
        ],
        false,
    );
    let formula = format!("CONCAT({})", vec!["\"a\""; 253].join(","));
    assert_formulas(&[&formula], &[CellValue::Text("a".repeat(253))], false);
}

#[test]
fn concat_sparse_full_columns_keep_argument_order_and_materialize_spills() {
    let xml = sheet(
        r#"<row r="1"><c r="B1" t="inlineStr"><is><t>a</t></is></c><c r="C1" t="inlineStr"><is><t>B</t></is></c><c r="F1"><f>CONCAT(B:B,C:C)</f></c><c r="G1"><f>CONCAT(B3#)</f></c></row><row r="3"><c r="B3"><f>SEQUENCE(2,1,7)</f></c></row><row r="1048576"><c r="B1048576" t="inlineStr"><is><t>z</t></is></c></row>"#,
    );
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    assert_eq!(
        computed(&result, 0, "F1"),
        Some(&CellValue::Text("a78zB".into()))
    );
    assert_eq!(
        computed(&result, 0, "G1"),
        Some(&CellValue::Text("78".into()))
    );
}

#[test]
fn text_broadcasts_value_and_format_arrays_and_returns_text_for_booleans() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(TEXT({12,34},{\"000\";\"0.00\"}),1)",
            "TEXT(\"12\",\"000\")",
            "TEXT(TRUE,\"0\")",
            "ISTEXT(TEXT(TRUE,\"0\"))",
            "ARRAYTOTEXT(TEXT({TRUE,FALSE,\"abc\",#N/A},\"0\"),1)",
            "ARRAYTOTEXT(TEXT(12,{\"000\",#N/A}),1)",
            "TEXT({#N/A},\"0\")",
            "ROWS(A7#)",
            "TEXT(\"NaN\",\"0\")",
            "TEXT(\"inf\",\"0\")",
        ],
        &[
            CellValue::Text("{\"012\",\"034\";\"12.00\",\"34.00\"}".into()),
            CellValue::Text("012".into()),
            CellValue::Text("TRUE".into()),
            CellValue::Bool(true),
            CellValue::Text("{\"TRUE\",\"FALSE\",\"abc\",#N/A}".into()),
            CellValue::Text("{\"012\",#N/A}".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
            CellValue::Text("NaN".into()),
            CellValue::Text("inf".into()),
        ],
        false,
    );
}

#[test]
fn text_ranges_keep_format_errors_local_and_blank_values_numeric() {
    let xml = sheet(
        r##"<row r="1"><c r="B1"><v>12</v></c><c r="D1" t="inlineStr"><is><t>000</t></is></c><c r="F1"><f>TEXT(B1:B3,D1:D3)</f></c></row><row r="2"><c r="B2"><v>34</v></c><c r="D2" t="e"><v>#N/A</v></c></row><row r="3"><c r="D3" t="inlineStr"><is><t>0.00</t></is></c></row>"##,
    );
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (at, value) in [
        ("F1", CellValue::Text("012".into())),
        ("F2", CellValue::Error("#N/A".into())),
        ("F3", CellValue::Text("0.00".into())),
    ] {
        assert_eq!(computed_or_spilled(&result, at), Some(&value), "{at}");
    }
}

#[test]
fn scalar_errors_broadcast_across_text_function_array_arguments() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(LEFT({\"a\",\"b\"},NA()),1)",
            "ARRAYTOTEXT(REPT(NA(),{1,2}),1)",
            "ARRAYTOTEXT(EXACT({\"a\",\"b\"},NA()),1)",
            "ARRAYTOTEXT(FIXED({1,2},NA()),1)",
            "ARRAYTOTEXT(DOLLAR(NA(),{0,2}),1)",
            "ARRAYTOTEXT(REPLACE({\"a\",\"b\"},1,1,NA()),1)",
            "ARRAYTOTEXT(SUBSTITUTE({\"a\",\"b\"},\"a\",NA()),1)",
            "ARRAYTOTEXT(NUMBERVALUE({\"1\",\"2\"},NA()),1)",
            "ARRAYTOTEXT(FIND(NA(),{\"a\",\"b\"}),1)",
            "ARRAYTOTEXT(SEARCH({\"a\",\"b\"},NA()),1)",
        ],
        &vec![CellValue::Text("{#N/A,#N/A}".into()); 10],
        false,
    );
}

#[test]
fn scalar_error_broadcast_retains_shape_for_spill_references() {
    let xml = formula_sheet(&["REPT(NA(),{1,2;3,4})"]);
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for at in ["A1", "B1", "A2", "B2"] {
        assert_eq!(
            computed_or_spilled(&result, at),
            Some(&CellValue::Error("#N/A".into())),
            "{at}"
        );
    }
    assert_formulas(
        &[
            "REPT(NA(),{1,2})",
            "COLUMNS(A1#)",
            "ROWS(A1#)",
            "SUM(IFERROR(A1#,1))",
        ],
        &[
            CellValue::Error("#N/A".into()),
            CellValue::Number(2.0),
            CellValue::Number(1.0),
            CellValue::Number(2.0),
        ],
        false,
    );
}

#[test]
fn fixed_and_dollar_broadcast_numeric_decimal_and_grouping_arguments() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(FIXED({1234.567,-1234.567},{0;2},TRUE),1)",
            "ARRAYTOTEXT(FIXED(1234.5,2,{FALSE,TRUE,\"true\",\"bad\",#N/A}),1)",
            "ARRAYTOTEXT(DOLLAR({1234.567,-1234.567},{0;2}),1)",
            "ARRAYTOTEXT(FIXED({\"12.5\",TRUE,\"bad\",#N/A}),1)",
            "FIXED({#N/A})",
            "ROWS(A5#)",
            "DOLLAR({#N/A})",
            "ROWS(A7#)",
        ],
        &[
            CellValue::Text("{\"1235\",\"-1235\";\"1234.57\",\"-1234.57\"}".into()),
            CellValue::Text("{\"1,234.50\",\"1234.50\",\"1234.50\",#VALUE!,#N/A}".into()),
            CellValue::Text("{\"$1,235\",\"($1,235)\";\"$1,234.57\",\"($1,234.57)\"}".into()),
            CellValue::Text("{\"12.50\",\"1.00\",#VALUE!,#N/A}".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
        ],
        false,
    );
}

#[test]
fn fixed_and_dollar_array_decimal_errors_do_not_discard_other_elements() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(FIXED(1,{0,2,128,#N/A},TRUE),1)",
            "ARRAYTOTEXT(DOLLAR(1,{0,2,128,#N/A}),1)",
            "ARRAYTOTEXT(FIXED({1,2},,TRUE),1)",
            "ARRAYTOTEXT(DOLLAR({1,2},),1)",
        ],
        &[
            CellValue::Text("{\"1\",\"1.00\",#VALUE!,#N/A}".into()),
            CellValue::Text("{\"$1\",\"$1.00\",#VALUE!,#N/A}".into()),
            CellValue::Text("{\"1\",\"2\"}".into()),
            CellValue::Text("{\"$1\",\"$2\"}".into()),
        ],
        false,
    );
}

#[test]
fn fixed_and_dollar_preserve_large_numbers_without_integer_saturation() {
    assert_formulas(
        &[
            "FIXED(1E20,2,TRUE)",
            "DOLLAR(1E20,0)",
            "FIXED(1E308,2,TRUE)",
            "FIXED(1E-100,127,TRUE)",
            "FIXED(1234.5,-1.9)",
            "DOLLAR(-1234.5,-1.9)",
        ],
        &[
            CellValue::Text("100000000000000000000.00".into()),
            CellValue::Text("$100,000,000,000,000,000,000".into()),
            CellValue::Text(format!("1{}.00", "0".repeat(308))),
            CellValue::Text(format!("0.{}1{}", "0".repeat(99), "0".repeat(27))),
            CellValue::Text("1,230".into()),
            CellValue::Text("($1,230)".into()),
        ],
        false,
    );
}

#[test]
fn fixed_and_dollar_round_decimal_digits_and_bound_decimal_counts() {
    assert_formulas(
        &[
            "FIXED(1.005,2)",
            "FIXED(9.999,2)",
            "FIXED(0.1+0.2,20,TRUE)",
            "LEN(FIXED(1,127,TRUE))",
            "FIXED(1,128)",
            "DOLLAR(1,1E100)",
            "FIXED(123,-1E100)",
            "DOLLAR(9.995,2)",
        ],
        &[
            CellValue::Text("1.01".into()),
            CellValue::Text("10.00".into()),
            CellValue::Text("0.30000000000000000000".into()),
            CellValue::Number(129.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Text("0".into()),
            CellValue::Text("$10.00".into()),
        ],
        false,
    );
}

#[test]
fn numbervalue_broadcasts_text_and_separator_arrays_with_element_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(NUMBERVALUE({\"1,5\",\"1.5\"},{\",\";\".\"},\" \"),1)",
            "ARRAYTOTEXT(NUMBERVALUE({\"1.234,5\",\"1,234.5\"},{\",\",\".\"},{\".\",\",\"}),1)",
            "ARRAYTOTEXT(NUMBERVALUE({\"9%%\",\"\",\"bad\",#N/A,12}),1)",
            "NUMBERVALUE({\"bad\"})",
            "ROWS(A4#)",
            "SUM(IFERROR(NUMBERVALUE({\"1.5\",\"bad\",\"2.5\"}),0))",
        ],
        &[
            CellValue::Text("{1.5,#VALUE!;#VALUE!,1.5}".into()),
            CellValue::Text("{1234.5,1234.5}".into()),
            CellValue::Text("{0.0009,0,#VALUE!,#N/A,12}".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(1.0),
            CellValue::Number(4.0),
        ],
        false,
    );
}

#[test]
fn numbervalue_array_defaults_and_invalid_separators_stay_per_element() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(NUMBERVALUE({\"1,234.5\",\"2.5\"},,),1)",
            "ARRAYTOTEXT(NUMBERVALUE({\"1·5\",\"2点5\"},{\"·more\",\"点more\"},\" \"),1)",
            "ARRAYTOTEXT(NUMBERVALUE(\"1.5\",{\".\",\"\",#N/A},\",\"),1)",
            "ARRAYTOTEXT(NUMBERVALUE(\"1.5\",\".\",{\",\",\".\",#N/A}),1)",
        ],
        &[
            CellValue::Text("{1234.5,2.5}".into()),
            CellValue::Text("{1.5,2.5}".into()),
            CellValue::Text("{1.5,#VALUE!,#N/A}".into()),
            CellValue::Text("{1.5,#VALUE!,#N/A}".into()),
        ],
        false,
    );
}

#[test]
fn search_handles_many_distinct_patterns_and_keeps_original_offsets() {
    assert_formulas(
        &[
            "SUM(SEARCH(REPT(\"a\",SEQUENCE(20)),\"😀\"&REPT(\"A\",20)))",
            "SUM(IFERROR(SEARCH(REPT(\"z\",SEQUENCE(20)),\"abc\"),0))",
        ],
        &[CellValue::Number(40.0), CellValue::Number(0.0)],
        false,
    );
}

#[test]
fn repeated_search_patterns_work_over_large_spilled_text_arrays() {
    assert_formulas(
        &[
            "SUM(SEARCH(\"a*\",REPT(\"AB\",SEQUENCE(10000,1,1,0))))",
            "SUM(SEARCH({\"a*\";\"b*\"},REPT(\"AB\",SEQUENCE(1,1000,1,0))))",
        ],
        &[CellValue::Number(10000.0), CellValue::Number(3000.0)],
        false,
    );
}

#[test]
fn search_broadcasts_wildcards_and_start_positions() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(SEARCH({\"A\",\"?b\",\"~*\",#N/A},\"aB*\"),1)",
            "ARRAYTOTEXT(SEARCH({\"a\",\"B\"},{\"ABC\";\"bca\"}),1)",
            "ARRAYTOTEXT(SEARCH(\"a\",\"AbAbA\",{1,2,4,0,1E100,#N/A}),1)",
            "SEARCH({\"z\"},\"abc\")",
            "ROWS(A4#)",
            "SUM(IFERROR(SEARCH({\"A\",\"z\"},\"abc\"),0))",
        ],
        &[
            CellValue::Text("{1,1,3,#N/A}".into()),
            CellValue::Text("{1,2;3,1}".into()),
            CellValue::Text("{1,3,5,#VALUE!,#VALUE!,#N/A}".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(1.0),
            CellValue::Number(1.0),
        ],
        false,
    );
}

#[test]
fn search_keeps_original_unicode_positions_and_matches_newline_wildcards() {
    assert_formulas(
        &[
            "SEARCH(\"b\",\"İB\")",
            "SEARCH(\"B\",\"İb\",2)",
            "SEARCH(\"b\",\"a😀B\")",
            "SEARCH(\"a?b\",\"A\"&CHAR(10)&\"B\")",
            "SEARCH(\"a*b\",\"A\"&CHAR(10)&\"B\")",
            "SEARCH(\"~?\",\"a?b\")",
            "SEARCH(\"~~\",\"a~b\")",
            "SEARCH(\"本\",\"日本語\")",
        ],
        &[
            CellValue::Number(2.0),
            CellValue::Number(2.0),
            CellValue::Number(3.0),
            CellValue::Number(1.0),
            CellValue::Number(1.0),
            CellValue::Number(2.0),
            CellValue::Number(2.0),
            CellValue::Number(2.0),
        ],
        false,
    );
}

#[test]
fn find_broadcasts_search_text_and_start_positions() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(FIND({\"a\",\"B\",\"*\",#N/A},\"aBa*\"),1)",
            "ARRAYTOTEXT(FIND({\"a\",\"b\"},{\"abc\";\"bca\"}),1)",
            "ARRAYTOTEXT(FIND(\"a\",\"ababa\",{1,2,4,0,1E100,#N/A}),1)",
            "FIND({\"x\"},\"abc\")",
            "ROWS(A4#)",
            "SUM(IFERROR(FIND({\"a\",\"z\"},\"abc\"),0))",
        ],
        &[
            CellValue::Text("{1,2,4,#N/A}".into()),
            CellValue::Text("{1,2;3,1}".into()),
            CellValue::Text("{1,3,5,#VALUE!,#VALUE!,#N/A}".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(1.0),
            CellValue::Number(1.0),
        ],
        false,
    );
}

#[test]
fn find_preserves_unicode_positions_case_and_literal_wildcards() {
    assert_formulas(
        &[
            "FIND(\"本\",\"日本語\")",
            "FIND(\"b\",\"a😀b\")",
            "FIND(\"😀\",\"a😀b😀\",3)",
            "FIND(\"A\",\"abc\")",
            "FIND(\"?\",\"a?b\")",
            "FIND(\"\",\"abc\",2)",
            "FIND(2,123)",
            "FIND(\"a\",\"aba\",\"2\")",
            "FIND(\"a\",\"aba\",2.9)",
        ],
        &[
            CellValue::Number(2.0),
            CellValue::Number(3.0),
            CellValue::Number(4.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(2.0),
            CellValue::Number(2.0),
            CellValue::Number(2.0),
            CellValue::Number(3.0),
            CellValue::Number(3.0),
        ],
        false,
    );
}

#[test]
fn substitute_broadcasts_occurrence_selection_and_counts_nonoverlapping_matches() {
    assert_formulas(
        &[
            "SUBSTITUTE(\"aaaa\",\"aa\",\"X\",2)",
            "SUBSTITUTE(\"aaaa\",\"aa\",\"X\")",
            "SUBSTITUTE(\"aaaa\",\"aa\",\"X\",3)",
            "ARRAYTOTEXT(SUBSTITUTE({\"a-a\",\"b-b\"},{\"a\";\"b\"},\"X\"),1)",
            "ARRAYTOTEXT(SUBSTITUTE(\"a-a-a\",\"a\",\"X\",{1,2,3,4,0,#N/A}),1)",
            "SUBSTITUTE({#N/A},\"a\",\"X\")",
            "ROWS(A6#)",
        ],
        &[
            CellValue::Text("aaX".into()),
            CellValue::Text("XX".into()),
            CellValue::Text("aaaa".into()),
            CellValue::Text("{\"X-X\",\"b-b\";\"a-a\",\"X-X\"}".into()),
            CellValue::Text("{\"X-a-a\",\"a-X-a\",\"a-a-X\",\"a-a-a\",#VALUE!,#N/A}".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
        ],
        false,
    );
}

#[test]
fn substitute_enforces_output_limits_and_handles_empty_search() {
    assert_formulas(
        &[
            "SUBSTITUTE(\"abc\",\"\",\"X\")",
            "SUBSTITUTE(\"abc\",\"a\",\"X\",1E100)",
            "SUBSTITUTE(REPT(\"a\",32767),\"a\",\"aa\")",
            "LEN(SUBSTITUTE(REPT(\"a\",32767),\"a\",\"日\"))",
            "SUBSTITUTE(\"a\",\"a\",REPT(\"日\",32767))",
            "SUBSTITUTE(\"abc\",\"a\",\"X\",0)",
            "SUBSTITUTE(\"AaA\",\"a\",\"X\")",
        ],
        &[
            CellValue::Text("abc".into()),
            CellValue::Text("abc".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(32767.0),
            CellValue::Text("日".repeat(32767)),
            CellValue::Error("#VALUE!".into()),
            CellValue::Text("AXA".into()),
        ],
        false,
    );
}

#[test]
fn replace_broadcasts_all_arguments_and_preserves_element_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(REPLACE({\"abc\",\"def\"},{1;2},1,{\"X\",\"Y\"}),1)",
            "ARRAYTOTEXT(REPLACE(\"abc\",1,{0,1,-1,#N/A},\"X\"),1)",
            "REPLACE({#N/A},1,1,\"x\")",
            "ROWS(A3#)",
            "REPLACE(2009,3,2,10)",
            "REPLACE(\"a😀b\",2,1,\"X\")",
            "REPLACE(\"abc\",\"2\",TRUE,12)",
        ],
        &[
            CellValue::Text("{\"Xbc\",\"Yef\";\"aXc\",\"dYf\"}".into()),
            CellValue::Text("{\"Xabc\",\"Xbc\",#VALUE!,#N/A}".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
            CellValue::Text("2010".into()),
            CellValue::Text("aXb".into()),
            CellValue::Text("a12c".into()),
        ],
        false,
    );
}

#[test]
fn replace_limits_output_and_handles_large_positions_and_lengths() {
    assert_formulas(
        &[
            "REPLACE(\"abc\",1E100,1E100,\"X\")",
            "REPLACE(\"abc\",2,1E100,\"X\")",
            "LEN(REPLACE(REPT(\"日\",32767),1,1,\"本\"))",
            "REPLACE(REPT(\"日\",32767),1,0,\"本\")",
            "REPLACE(\"abc\",0,1,\"X\")",
            "REPLACE(\"abc\",2,-1,\"X\")",
            "REPLACE(\"abcdef\",2.9,2.9,\"X\")",
        ],
        &[
            CellValue::Text("abcX".into()),
            CellValue::Text("aX".into()),
            CellValue::Number(32767.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Text("aXdef".into()),
        ],
        false,
    );
}

#[test]
fn proper_maps_word_boundaries_and_keeps_array_errors() {
    assert_formulas(
        &["ARRAYTOTEXT(PROPER({\"this is a TITLE\",\"2-way STREET\",\"76BudGet\",\"o'NEILL\",\"日本語 abc\",#N/A}),1)", "ARRAYTOTEXT(PROPER({TRUE,12,\"\"}),1)", "PROPER({#N/A})", "ROWS(A3#)", "SUM(--EXACT(PROPER({\"a\",\"b\"}),{\"A\",\"B\"}))", "ARRAYTOTEXT(LOWER(UPPER({\"AbC\",#N/A})),1)"],
        &[CellValue::Text("{\"This Is A Title\",\"2-Way Street\",\"76Budget\",\"O'Neill\",\"日本語 Abc\",#N/A}".into()), CellValue::Text("{\"True\",\"12\",\"\"}".into()), CellValue::Error("#N/A".into()), CellValue::Number(1.0), CellValue::Number(2.0), CellValue::Text("{\"abc\",#N/A}".into())],
        false,
    );
}

#[test]
fn proper_ranges_preserve_empty_cells_and_literal_error_text() {
    let xml = sheet(
        r##"<row r="1"><c r="B1" t="inlineStr"><is><t>  hELLO   WORLD  </t></is></c><c r="C1"><f>""</f></c><c r="E1" t="inlineStr"><is><t>#N/A</t></is></c><c r="G1"><f>PROPER(B1:E1)</f></c></row>"##,
    );
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (at, text) in [
        ("G1", "  Hello   World  "),
        ("H1", ""),
        ("I1", ""),
        ("J1", "#N/A"),
    ] {
        assert_eq!(
            computed_or_spilled(&result, at),
            Some(&CellValue::Text(text.into())),
            "{at}"
        );
    }
}

#[test]
fn unary_array_operations_coerce_each_value_and_keep_error_shapes() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(-{1,TRUE,FALSE,\"2\",\"bad\",#N/A},1)",
            "ARRAYTOTEXT({50,TRUE,\"25\",#N/A}%,1)",
            "SUM(--{TRUE,FALSE,TRUE})",
            "-{#N/A}",
            "ROWS(A4#)",
        ],
        &[
            CellValue::Text("{-1,-1,0,-2,#VALUE!,#N/A}".into()),
            CellValue::Text("{0.5,0.01,0.25,#N/A}".into()),
            CellValue::Number(2.0),
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
        ],
        false,
    );
}

#[test]
fn exact_broadcasts_case_sensitive_comparisons_and_preserves_errors() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(EXACT({\"word\",\"Word\",\"w ord\",#N/A},\"word\"),1)",
            "ARRAYTOTEXT(EXACT({\"A\",\"B\"},{\"A\";\"B\"}),1)",
            "ARRAYTOTEXT(EXACT({12,TRUE,\"\",\"😀\"},{\"12\",\"TRUE\",\"\",\"😀\"}),1)",
            "EXACT({#N/A},\"x\")",
            "ROWS(A4#)",
            "SUM(--EXACT({\"A\",\"a\",\"A\"},\"A\"))",
        ],
        &[
            CellValue::Text("{TRUE,FALSE,FALSE,#N/A}".into()),
            CellValue::Text("{TRUE,FALSE;FALSE,TRUE}".into()),
            CellValue::Text("{TRUE,TRUE,TRUE,TRUE}".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
            CellValue::Number(2.0),
        ],
        false,
    );
}

#[test]
fn exact_range_compares_blank_cells_and_empty_formula_text() {
    let xml = sheet(
        r##"<row r="1"><c r="B1" t="inlineStr"><is><t>#N/A</t></is></c><c r="C1"><f>""</f></c><c r="E1" t="e"><v>#N/A</v></c><c r="G1"><f>EXACT(B1:E1,"#N/A")</f></c></row><row r="2"><c r="G2"><f>EXACT(B1:E1,"")</f></c></row>"##,
    );
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (at, value) in [
        ("G1", CellValue::Bool(true)),
        ("H1", CellValue::Bool(false)),
        ("I1", CellValue::Bool(false)),
        ("J1", CellValue::Error("#N/A".into())),
        ("G2", CellValue::Bool(false)),
        ("H2", CellValue::Bool(true)),
        ("I2", CellValue::Bool(true)),
        ("J2", CellValue::Error("#N/A".into())),
    ] {
        assert_eq!(computed_or_spilled(&result, at), Some(&value), "{at}");
    }
}

#[test]
fn unichar_distinguishes_surrogates_from_out_of_range_numbers() {
    assert_formulas(
        &[
            "UNICHAR(55296)",
            "UNICHAR(56319)",
            "UNICHAR(56320)",
            "UNICHAR(57343)",
            "UNICODE(UNICHAR(55295))",
            "UNICODE(UNICHAR(57344))",
            "UNICODE(UNICHAR(1114111))",
            "UNICHAR(1114112)",
            "UNICHAR(0)",
            "UNICHAR(-1)",
            "UNICHAR(1E100)",
            "UNICHAR(128512)",
            "UNICHAR(66.9)",
        ],
        &[
            CellValue::Error("#N/A".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Number(55295.0),
            CellValue::Number(57344.0),
            CellValue::Number(1114111.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Text("😀".into()),
            CellValue::Text("B".into()),
        ],
        false,
    );
}

#[test]
fn character_constructors_map_arrays_and_preserve_error_spills() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(UNICHAR({65,128512,55296,0,#DIV/0!,\"66\"}),1)",
            "ARRAYTOTEXT(CHAR({65,128,255,256,0,#N/A}),1)",
            "UNICHAR({55296})",
            "ROWS(A3#)",
            "SUM(IFERROR(UNICODE(UNICHAR({65,66,0})),0))",
        ],
        &[
            CellValue::Text("{\"A\",\"😀\",#N/A,#VALUE!,#DIV/0!,\"B\"}".into()),
            CellValue::Text("{\"A\",\"€\",\"ÿ\",#VALUE!,#VALUE!,#N/A}".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
            CellValue::Number(131.0),
        ],
        false,
    );
}

#[test]
fn clean_and_unicode_map_arrays_and_compose_with_trim() {
    assert_formulas(
        &[
            "ARRAYTOTEXT(CLEAN({\" a \",12,TRUE,#N/A}),1)",
            "CLEAN(CHAR(9)&\"  Monthly  report  \"&CHAR(10))",
            "TRIM(CLEAN(CHAR(9)&\"  Monthly  report  \"&CHAR(10)))",
            "CLEAN(UNICHAR(127)&UNICHAR(129)&UNICHAR(160))",
            "ARRAYTOTEXT(UNICODE({\"A\",\"日本\",\"😀\",\"\",#N/A,12,TRUE}),1)",
            "UNICODE({\"\"})",
            "ROWS(A6#)",
            "SUM(IFERROR(UNICODE({\"A\",\"B\",\"\"}),0))",
        ],
        &[
            CellValue::Text("{\" a \",\"12\",\"TRUE\",#N/A}".into()),
            CellValue::Text("  Monthly  report  ".into()),
            CellValue::Text("Monthly report".into()),
            CellValue::Text("\u{7f}\u{81}\u{a0}".into()),
            CellValue::Text("{65,26085,128512,#VALUE!,#N/A,49,84}".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(1.0),
            CellValue::Number(131.0),
        ],
        false,
    );
}

#[test]
fn clean_and_unicode_ranges_keep_blank_empty_and_error_values_distinct() {
    let xml = sheet(
        r##"<row r="1"><c r="B1" t="inlineStr"><is><t>_x0009_ A _x000A_</t></is></c><c r="C1"><f>""</f></c><c r="E1" t="e"><v>#N/A</v></c><c r="G1"><f>CLEAN(B1:E1)</f></c></row><row r="2"><c r="G2"><f>UNICODE(B1:E1)</f></c></row>"##,
    );
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (at, value) in [
        ("G1", CellValue::Text(" A ".into())),
        ("H1", CellValue::Text("".into())),
        ("I1", CellValue::Text("".into())),
        ("J1", CellValue::Error("#N/A".into())),
        ("G2", CellValue::Number(9.0)),
        ("H2", CellValue::Error("#VALUE!".into())),
        ("I2", CellValue::Error("#VALUE!".into())),
        ("J2", CellValue::Error("#N/A".into())),
    ] {
        assert_eq!(computed_or_spilled(&result, at), Some(&value), "{at}");
    }
}

#[test]
fn trim_maps_arrays_and_preserves_non_ascii_spaces_and_errors() {
    let xml = formula_sheet(&["TRIM({\"  a   b  \",\" \";TRUE,#N/A})"]);
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (at, value) in [
        ("A1", CellValue::Text("a b".into())),
        ("B1", CellValue::Text("".into())),
        ("A2", CellValue::Text("TRUE".into())),
        ("B2", CellValue::Error("#N/A".into())),
    ] {
        assert_eq!(computed_or_spilled(&result, at), Some(&value));
    }
    assert_formulas(
        &[
            "TRIM(\"  a   b　  c  \")",
            "TRIM(CHAR(9)&\"  a  \"&CHAR(10))",
            "TRIM(123)",
            "TRIM({#N/A})",
            "ROWS(A4#)",
            "SUM(LEN(TRIM({\" a \",\" b  c \"})))",
        ],
        &[
            CellValue::Text("a  b　 c".into()),
            CellValue::Text("\t a \n".into()),
            CellValue::Text("123".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
            CellValue::Number(4.0),
        ],
        false,
    );
}

#[test]
fn trim_range_keeps_blank_cells_empty_and_error_looking_text_typed() {
    let xml = sheet(
        r##"<row r="1"><c r="F1"><f>TRIM(B1:D1)</f></c><c r="B1" t="inlineStr"><is><t>  #N/A  </t></is></c><c r="C1"><f>""</f></c></row>"##,
    );
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    for (at, value) in [("F1", "#N/A"), ("G1", ""), ("H1", "")] {
        assert_eq!(
            computed_or_spilled(&result, at),
            Some(&CellValue::Text(value.into()))
        );
    }
}

#[test]
fn rept_counts_unicode_cell_units_and_rejects_large_counts_without_overflow() {
    assert_formulas(
        &[
            "REPT(\"*-\",3.9)",
            "LEN(REPT(\"日\",32767))",
            "REPT(\"日\",32768)",
            "LEN(REPT(\"😀\",16383))",
            "REPT(\"😀\",16384)",
            "REPT(\"abc\",1000000000)",
            "REPT(\"abc\",1E100)",
            "REPT(\"\",1E100)",
            "REPT(\"abc\",0)",
            "REPT(\"abc\",-0.5)",
            "REPT(12,\"2\")",
            "REPT(TRUE,TRUE)",
            "REPT(NA(),0)",
        ],
        &[
            CellValue::Text("*-*-*-".into()),
            CellValue::Number(32767.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(16383.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Text("".into()),
            CellValue::Text("".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Text("1212".into()),
            CellValue::Text("TRUE".into()),
            CellValue::Error("#N/A".into()),
        ],
        false,
    );
}

#[test]
fn rept_broadcasts_text_and_count_arrays_with_per_element_errors() {
    let xml = formula_sheet(&["REPT({\"a\",\"日\",#N/A,\"\"},{1;2;3})"]);
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for row in 1..=3 {
        for (column, value) in [
            ("A", CellValue::Text("a".repeat(row))),
            ("B", CellValue::Text("日".repeat(row))),
            ("C", CellValue::Error("#N/A".into())),
            ("D", CellValue::Text(String::new())),
        ] {
            assert_eq!(
                computed_or_spilled(&result, &format!("{column}{row}")),
                Some(&value)
            );
        }
    }
    assert_formulas(
        &["ARRAYTOTEXT(REPT(\"x\",{0,2,-1,#N/A,\"3\",TRUE}),1)"],
        &[CellValue::Text(
            "{\"\",\"xx\",#VALUE!,#N/A,\"xxx\",\"x\"}".into(),
        )],
        false,
    );
    assert_formulas(
        &[
            "REPT({#N/A},2)",
            "ROWS(A1#)",
            "REPT(\"x\",{2})",
            "ARRAYTOTEXT(A3#,1)",
        ],
        &[
            CellValue::Error("#N/A".into()),
            CellValue::Number(1.0),
            CellValue::Text("xx".into()),
            CellValue::Text("{\"xx\"}".into()),
        ],
        false,
    );
}

#[test]
fn len_maps_arrays_without_losing_errors_or_text_coercion() {
    let xml = formula_sheet(&[
        "LEN({\"a\"\"b\",\"\",12,TRUE,FALSE,#N/A})",
        "SUM(IFERROR(A5#,0))",
        "LEN({\"😀\",\"✈️\",\"é\"})",
        "0",
        "LEN({#VALUE!,\"abc\";\" x \",\"日本語\"})",
    ]);
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (at, value) in [
        ("A1", CellValue::Number(3.0)),
        ("B1", CellValue::Number(0.0)),
        ("C1", CellValue::Number(2.0)),
        ("D1", CellValue::Number(4.0)),
        ("E1", CellValue::Number(5.0)),
        ("F1", CellValue::Error("#N/A".into())),
        ("A2", CellValue::Number(9.0)),
        ("A3", CellValue::Number(1.0)),
        ("B3", CellValue::Number(2.0)),
        ("C3", CellValue::Number(2.0)),
        ("A5", CellValue::Error("#VALUE!".into())),
        ("B5", CellValue::Number(3.0)),
        ("A6", CellValue::Number(3.0)),
        ("B6", CellValue::Number(3.0)),
    ] {
        assert_eq!(computed_or_spilled(&result, at), Some(&value), "{at}");
    }
}

#[test]
fn len_range_distinguishes_blank_empty_text_and_error_cells() {
    let xml = sheet(
        r#"<row r="1"><c r="F1"><f>LEN(B1:D1)</f></c><c r="B1" t="inlineStr"><is><t> x </t></is></c><c r="C1"><f>""</f></c></row><row r="2"><c r="B2" t="e"><v>#N/A</v></c><c r="C2" t="b"><v>1</v></c><c r="D2"><v>123</v></c><c r="F2"><f>LEN(B2:D2)</f></c></row>"#,
    );
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (at, expected) in [("F1", 3.0), ("G1", 0.0), ("H1", 0.0)] {
        assert_eq!(
            computed_or_spilled(&result, at),
            Some(&CellValue::Number(expected)),
            "{at}"
        );
    }
    assert_eq!(
        computed_or_spilled(&result, "F2"),
        Some(&CellValue::Error("#N/A".into()))
    );
    assert_eq!(
        computed_or_spilled(&result, "G2"),
        Some(&CellValue::Number(4.0))
    );
    assert_eq!(
        computed_or_spilled(&result, "H2"),
        Some(&CellValue::Number(3.0))
    );
}

#[test]
fn array_constant_quotes_are_decoded_once_during_evaluation() {
    let xml = formula_sheet(&[
        "{\"a\"\"b\",\"\"\"\"\"\"}",
        "HSTACK(LEN(INDEX({\"a\"\"b\",\"\"\"\"\"\"},1,1)),LEN(INDEX({\"a\"\"b\",\"\"\"\"\"\"},1,2)))",
        "VALUETOTEXT({\"a\"\"b\",\"\"\"\"\"\"},1)",
        "BYROW({\"\"\"\"\"\"},LAMBDA(r,INDEX(r,1,1)))",
        "BYCOL({\"\"\"\"\"\"},LAMBDA(c,INDEX(c,1,1)))",
    ]);
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (at, value) in [
        ("A1", CellValue::Text("a\"b".into())),
        ("B1", CellValue::Text("\"\"".into())),
        ("A2", CellValue::Number(3.0)),
        ("B2", CellValue::Number(2.0)),
        ("A3", CellValue::Text("\"a\"\"b\"".into())),
        ("B3", CellValue::Text("\"\"\"\"\"\"".into())),
        ("A4", CellValue::Text("\"\"".into())),
        ("A5", CellValue::Text("\"\"".into())),
    ] {
        assert_eq!(computed_or_spilled(&result, at), Some(&value), "{at}");
    }
}

#[test]
fn arraytotext_preserves_shape_and_enforces_the_utf16_cell_limit() {
    assert_formulas(
        &[
            "ARRAYTOTEXT({1,\"a\"\"b\";TRUE,#N/A},1)",
            "ARRAYTOTEXT({1,\"a\"\"b\";TRUE,#N/A},0)",
            "ARRAYTOTEXT({1,2},0.5)",
            "ARRAYTOTEXT({1,2},1.5)",
            "LEN(ARRAYTOTEXT({1,2},))",
            "LEN(ARRAYTOTEXT(REPT(\"a\",32767)))",
            "ARRAYTOTEXT(REPT(\"a\",32766),1)",
            "LEN(ARRAYTOTEXT({\"a\",\"b\"},1))",
            "ARRAYTOTEXT(REPT(\"😀\",16384))",
            "ARRAYTOTEXT(SEQUENCE(10000))",
            "ARRAYTOTEXT(Z1:Z1048576)",
            "ARRAYTOTEXT(Z1:Z1048576,1)",
        ],
        &[
            CellValue::Text("{1,\"a\"\"b\";TRUE,#N/A}".into()),
            CellValue::Text("1, a\"b, TRUE, #N/A".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(4.0),
            CellValue::Number(32767.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(9.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
        ],
        false,
    );
}

#[test]
fn error_looking_formula_strings_are_text_and_real_errors_keep_their_type() {
    assert_formulas(
        &[
            "\"#N/A\"",
            "\"#VALUE!\"",
            "\"#NAME?\"",
            "\"#ERROR!\"",
            "NA()",
            "1/0",
        ],
        &[
            CellValue::Text("#N/A".into()),
            CellValue::Text("#VALUE!".into()),
            CellValue::Text("#NAME?".into()),
            CellValue::Text("#ERROR!".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Error("#DIV/0!".into()),
        ],
        false,
    );
}

#[test]
fn valuetotext_spills_text_values_and_escapes_only_text_in_strict_mode() {
    let xml = formula_sheet(&[
        "VALUETOTEXT({1,\"1\",TRUE,#N/A,\"#N/A\"},1)",
        "VALUETOTEXT({1,\"1\",TRUE,#N/A,\"#N/A\"},0)",
    ]);
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (row, values) in [
        ["1", "\"1\"", "TRUE", "#N/A", "\"#N/A\""],
        ["1", "1", "TRUE", "#N/A", "#N/A"],
    ]
    .iter()
    .enumerate()
    {
        for (column, value) in values.iter().enumerate() {
            let at = format!("{}{}", (b'A' + column as u8) as char, row + 1);
            assert_eq!(
                computed_or_spilled(&result, &at),
                Some(&CellValue::Text((*value).into())),
                "{at}"
            );
        }
    }
    assert_formulas(
        &[
            "LEN(VALUETOTEXT(REPT(\"a\",32765),1))",
            "VALUETOTEXT(REPT(\"a\",32766),1)",
        ],
        &[
            CellValue::Number(32767.0),
            CellValue::Error("#VALUE!".into()),
        ],
        false,
    );
}

#[test]
fn valuetotext_accepts_concise_and_strict_formats_without_losing_value_types() {
    assert_formulas(
        &[
            "VALUETOTEXT(\"Hello\",0)",
            "VALUETOTEXT(\"Hello\",1)",
            "VALUETOTEXT(\"a\"\"b\",1)",
            "VALUETOTEXT(TRUE,1)",
            "VALUETOTEXT(1234.01234,1)",
            "VALUETOTEXT(#VALUE!,1)",
            "VALUETOTEXT(\"x\",2)",
            "VALUETOTEXT(\"x\",0.5)",
            "VALUETOTEXT(\"x\",#N/A)",
            "VALUETOTEXT(\"x\",)",
            "ISERROR(VALUETOTEXT(#N/A,1))",
            "ISTEXT(VALUETOTEXT(#N/A,1))",
        ],
        &[
            CellValue::Text("Hello".into()),
            CellValue::Text("\"Hello\"".into()),
            CellValue::Text("\"a\"\"b\"".into()),
            CellValue::Text("TRUE".into()),
            CellValue::Text("1234.01234".into()),
            CellValue::Text("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Text("x".into()),
            CellValue::Bool(false),
            CellValue::Bool(true),
        ],
        false,
    );
}

#[test]
fn value_time_and_datetime_arrays_keep_the_workbook_date_epoch() {
    let xml = formula_sheet(&["VALUE({\"1/1/1904\",\"1/1/1904 12:00\",\"1904-01-01T12:00\",\"12:00 AM\",\"12:00 PM\",\"9 PM\"})"]);
    for date1904 in [false, true] {
        let result = evaluate(&read_xlsx(&workbook(&xml, None, date1904)).unwrap()).unwrap();
        assert!(result.skipped.is_empty());
        let day = if date1904 { 0.0 } else { 1462.0 };
        for (column, value) in [day, day + 0.5, day + 0.5, 0.0, 0.5, 0.875]
            .into_iter()
            .enumerate()
        {
            let at = format!("{}1", (b'A' + column as u8) as char);
            assert_eq!(
                computed_or_spilled(&result, &at),
                Some(&CellValue::Number(value)),
                "{at}, date1904={date1904}"
            );
        }
    }
    assert_formulas(
        &["ROUND(VALUE(\"16:48:00\")-VALUE(\"12:00:00\"),10)"],
        &[CellValue::Number(0.2)],
        false,
    );
}

#[test]
fn value_converts_mixed_arrays_with_element_errors_and_original_blank_types() {
    let xml = sheet(
        r#"<row r="1"><c r="A1"><f>VALUE({"$1,000","12:00",7,TRUE,#N/A,"bad",""})</f></c></row><row r="3"><c r="B3" t="inlineStr"><is><t>42</t></is></c><c r="C3" t="str"><f>""</f><v></v></c></row><row r="6"><c r="A6"><f>VALUE(B3:D3)</f></c></row><row r="8"><c r="A8"><f>SUM(IFERROR(VALUE({"10","bad","20"}),0))</f></c></row>"#,
    );
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (cell, expected) in [
        ("A1", CellValue::Number(1000.0)),
        ("B1", CellValue::Number(0.5)),
        ("C1", CellValue::Number(7.0)),
        ("D1", CellValue::Error("#VALUE!".into())),
        ("E1", CellValue::Error("#N/A".into())),
        ("F1", CellValue::Error("#VALUE!".into())),
        ("G1", CellValue::Error("#VALUE!".into())),
        ("A6", CellValue::Number(42.0)),
        ("B6", CellValue::Error("#VALUE!".into())),
        ("C6", CellValue::Number(0.0)),
        ("A8", CellValue::Number(30.0)),
    ] {
        assert_eq!(
            computed_or_spilled(&result, cell),
            Some(&expected),
            "{cell}"
        );
    }
}

#[test]
fn numbervalue_obeys_documented_empty_space_separator_and_percent_rules() {
    assert_formulas(
        &[
            "NUMBERVALUE(\"\")",
            "NUMBERVALUE(\" 3 000 \")",
            "NUMBERVALUE(\"2.500,27\",\",decimal\",\".group\")",
            "NUMBERVALUE(\"1.2,3\")",
            "NUMBERVALUE(\"1.2\",\",\",\";\")",
            "NUMBERVALUE(\"9% %\")",
            "NUMBERVALUE(\"NaN\")",
            "NUMBERVALUE(\"inf\")",
            "NUMBERVALUE(\"1\",\"decimal\",\"duplicate\")",
            "NUMBERVALUE(\"1,2,3\")",
            "NUMBERVALUE(\"1.2.3\")",
            "NUMBERVALUE(\"1,234.5\",,\",\")",
            "NUMBERVALUE(\"1,234.5\",\".\",)",
            "NUMBERVALUE(\"1千234点5\",\"点余\",\"千余\")",
        ],
        &[
            CellValue::Number(0.0),
            CellValue::Number(3000.0),
            CellValue::Number(2500.27),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(0.0009),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(123.0),
            CellValue::Error("#VALUE!".into()),
            CellValue::Number(1234.5),
            CellValue::Number(1234.5),
            CellValue::Number(1234.5),
        ],
        false,
    );
}

#[test]
fn spill_references_keep_array_shape_when_the_first_element_is_an_error() {
    let xml = sheet(
        r#"<row r="1"><c r="A1"><f>{#N/A,2}</f></c><c r="D1"><f>SUM(IFERROR(A1#,0))</f></c></row><row r="3"><c r="A3"><f>{#N/A}</f></c><c r="D3"><f>ROWS(A3#)</f></c></row><row r="5"><c r="A5"><f>FILTER({1},{FALSE})</f></c><c r="D5"><f>IFERROR(ROWS(A5#),99)</f></c></row><row r="7"><c r="A7"><f>SEQUENCE(2)</f></c><c r="D7"><f>IFERROR(ROWS(A7#),98)</f></c></row><row r="8"><c r="A8"><v>1</v></c></row>"#,
    );
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (cell, expected) in [("D1", 2.0), ("D3", 1.0), ("D5", 99.0), ("D7", 98.0)] {
        assert_eq!(
            computed(&result, 0, cell),
            Some(&CellValue::Number(expected)),
            "{cell}"
        );
    }
}

#[test]
fn error_type_classifies_each_array_error_and_preserves_nonerror_na_results() {
    let result = evaluate(
        &read_xlsx(&workbook(
            &formula_sheet(&[
                "ERROR.TYPE({#NULL!,#DIV/0!,#VALUE!,#REF!,#NAME?,#NUM!,#N/A,1})",
                "ERROR.TYPE(1/{0,1,0})",
            ]),
            None,
            false,
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(result.skipped.is_empty());
    for column in 0..8 {
        let cell = format!("{}1", (b'A' + column) as char);
        let expected = if column < 7 {
            CellValue::Number(f64::from(column + 1))
        } else {
            CellValue::Error("#N/A".into())
        };
        assert_eq!(
            computed_or_spilled(&result, &cell),
            Some(&expected),
            "{cell}"
        );
    }
    for (cell, expected) in [
        ("A2", CellValue::Number(2.0)),
        ("B2", CellValue::Error("#N/A".into())),
        ("C2", CellValue::Number(2.0)),
    ] {
        assert_eq!(
            computed_or_spilled(&result, cell),
            Some(&expected),
            "{cell}"
        );
    }
}

#[test]
fn parity_functions_spill_and_keep_per_element_coercion_and_errors() {
    let result = evaluate(
        &read_xlsx(&workbook(
            &formula_sheet(&[
                "ISODD({1,-2.9,1E20,\"3\",TRUE,#N/A,\"bad\"})",
                "ISEVEN({1,-2.9,1E20,\"3\",TRUE,#N/A,\"bad\"})",
            ]),
            None,
            false,
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(result.skipped.is_empty());
    for row in 0..2 {
        for (column, odd) in [true, false, false, true].into_iter().enumerate() {
            let cell = format!("{}{}", (b'A' + column as u8) as char, row + 1);
            assert_eq!(
                computed_or_spilled(&result, &cell),
                Some(&CellValue::Bool(if row == 0 { odd } else { !odd })),
                "{cell}"
            );
        }
        for (column, error) in [('E', "#VALUE!"), ('F', "#N/A"), ('G', "#VALUE!")] {
            let cell = format!("{column}{}", row + 1);
            assert_eq!(
                computed_or_spilled(&result, &cell),
                Some(&CellValue::Error(error.into())),
                "{cell}"
            );
        }
    }
}

#[test]
fn sparse_textjoin_reads_newly_materialized_source_spills() {
    let xml = sheet(
        r#"<row r="1"><c r="A1"><f>TEXTJOIN(",",FALSE,B10:B12)</f></c></row><row r="2"><c r="A2"><f>TEXTJOIN(",",TRUE,C10:C12)</f></c></row><row r="3"><c r="A3"><f>TEXTJOIN(",",TRUE,D11:D12)</f></c></row><row r="4"><c r="A4"><f>TEXTJOIN("",FALSE,B1:XFD1048576)</f></c></row><row r="10"><c r="B10"><f>SEQUENCE(3)</f></c><c r="C10"><f>SEQUENCE(3)</f></c><c r="D10"><f>SEQUENCE(3)</f></c></row>"#,
    );
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (cell, expected) in [
        ("A1", "1,2,3"),
        ("A2", "1,2,3"),
        ("A3", "2,3"),
        ("A4", "111222333"),
    ] {
        assert_eq!(
            computed(&result, 0, cell),
            Some(&CellValue::Text(expected.into())),
            "{cell}"
        );
    }
}

#[test]
fn textjoin_keeps_the_logical_extent_of_sparse_rows_and_columns() {
    let formulas = [
        (
            "A5",
            "TEXTJOIN(\",\",FALSE,B:B)",
            CellValue::Error("#VALUE!".into()),
        ),
        (
            "A6",
            "TEXTJOIN(\"\",FALSE,B:B)",
            CellValue::Text("xy".into()),
        ),
        (
            "A7",
            "LEN(TEXTJOIN(\"-\",FALSE,1:1))",
            CellValue::Number(16384.0),
        ),
        (
            "A8",
            "LEN(TEXTJOIN(\"-\",FALSE,2:2))",
            CellValue::Number(16383.0),
        ),
        (
            "A9",
            "TEXTJOIN(\"\",TRUE,B:B)",
            CellValue::Text("xy".into()),
        ),
        (
            "A10",
            "TEXTJOIN(\"\",FALSE,B1:XFD1048576)",
            CellValue::Text("xy".into()),
        ),
        (
            "A11",
            "TEXTJOIN({\"\",\"-\"},FALSE,B1:B4,\"z\")",
            CellValue::Text("x-y-z".into()),
        ),
        (
            "A12",
            "LEN(TEXTJOIN({\"\",\"-\"},FALSE,2:2,\"z\"))",
            CellValue::Number(8193.0),
        ),
    ];
    let mut data = String::from(
        r#"<row r="1"><c r="B1" t="inlineStr"><is><t>x</t></is></c></row><row r="3"><c r="B3" t="inlineStr"><is><t>y</t></is></c></row>"#,
    );
    for (cell, formula, _) in &formulas {
        let row = &cell[1..];
        data.push_str(&format!(
            "<row r=\"{row}\"><c r=\"{cell}\"><f>{formula}</f></c></row>"
        ));
    }
    let result = evaluate(&read_xlsx(&workbook(&sheet(&data), None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (cell, _, expected) in formulas {
        assert_eq!(computed(&result, 0, cell), Some(&expected), "{cell}");
    }
}

#[test]
fn textjoin_cycles_array_delimiters_across_arguments_and_skipped_empty_values() {
    assert_formulas(&[
        "TEXTJOIN({\",\",\",\",\",\",\";\"},TRUE,{\"Tulsa\",\"OK\",74133,\"US\";\"Seattle\",\"WA\",98109,\"US\";\"end\",\"\",\"\",\"\"})",
        "TEXTJOIN({\"-\",\"/\"},TRUE,{\"a\",\"\",\"b\"},\"c\",\"d\")",
        "TEXTJOIN({\"-\";\"/\"},FALSE,{\"a\",\"\",\"b\"},\"c\")",
        "TEXTJOIN({1,TRUE,\"\"},TRUE,{\"a\",\"b\",\"c\",\"d\",\"e\"})",
        "TEXTJOIN({\"-\",#N/A},TRUE,{\"a\",\"b\",\"c\"})",
        "LEN(TEXTJOIN(HSTACK(\"-\",REPT(\"x\",32767)),TRUE,{\"a\",\"b\"}))",
        "TEXTJOIN(HSTACK(\"-\",REPT(\"x\",32767)),TRUE,{\"a\",\"b\",\"c\"})",
    ], &[
        CellValue::Text("Tulsa,OK,74133,US;Seattle,WA,98109,US;end".into()),
        CellValue::Text("a-b/c-d".into()),
        CellValue::Text("a-/b-c".into()),
        CellValue::Text("a1bTRUEcd1e".into()),
        CellValue::Error("#N/A".into()),
        CellValue::Number(3.0),
        CellValue::Error("#VALUE!".into()),
    ], false);
}

#[test]
fn textjoin_handles_matrix_values_empty_strings_errors_and_the_cell_limit() {
    assert_formulas(
        &[
            "TEXTJOIN(\"|\",TRUE,{\"a\",\"\",\"b\";\"c\",\"\",TRUE})",
            "TEXTJOIN(\"|\",FALSE,{\"a\",\"\",\"b\";\"c\",\"\",TRUE})",
            "TEXTJOIN(\"|\",TRUE,{\"a\",#DIV/0!})",
            "LEN(TEXTJOIN(\"\",FALSE,REPT(\"a\",32767)))",
            "TEXTJOIN(\"\",FALSE,REPT(\"a\",32767),\"b\")",
        ],
        &[
            CellValue::Text("a|b|c|TRUE".into()),
            CellValue::Text("a||b|c||TRUE".into()),
            CellValue::Error("#DIV/0!".into()),
            CellValue::Number(32767.0),
            CellValue::Error("#VALUE!".into()),
        ],
        false,
    );
}

#[test]
fn n_and_t_convert_each_array_value_and_preserve_element_errors() {
    let bytes = workbook(
        &formula_sheet(&[
            "N({2,\"2\",TRUE,FALSE,#N/A,#DIV/0!})",
            "T({2,\"2\",TRUE,FALSE,#N/A,#DIV/0!})",
        ]),
        None,
        false,
    );
    let result = evaluate(&read_xlsx(&bytes).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    let expected = [
        vec![
            CellValue::Number(2.0),
            CellValue::Number(0.0),
            CellValue::Number(1.0),
            CellValue::Number(0.0),
            CellValue::Error("#N/A".into()),
            CellValue::Error("#DIV/0!".into()),
        ],
        vec![
            CellValue::Text("".into()),
            CellValue::Text("2".into()),
            CellValue::Text("".into()),
            CellValue::Text("".into()),
            CellValue::Error("#N/A".into()),
            CellValue::Error("#DIV/0!".into()),
        ],
    ];
    for (row, values) in expected.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            let at = format!("{}{}", (b'A' + column as u8) as char, row + 1);
            assert_eq!(computed_or_spilled(&result, &at), Some(value), "{at}");
        }
    }
    assert_formulas(
        &[
            "SUM(IFERROR(N({2,\"2\",TRUE,#N/A}),10))",
            "TEXTJOIN(\"|\",FALSE,IFERROR(T({2,\"2\",TRUE,#N/A}),\"error\"))",
        ],
        &[CellValue::Number(13.0), CellValue::Text("|2||error".into())],
        false,
    );
}

fn computed_or_spilled<'a>(
    outcome: &'a jade_xlsx_calc::Recalculated,
    reference: &str,
) -> Option<&'a CellValue> {
    computed(outcome, 0, reference).or_else(|| {
        let reference = CellRef::parse(reference).unwrap();
        outcome
            .spills
            .iter()
            .flat_map(|spill| spill.cells.iter())
            .find(|cell| cell.sheet == 0 && cell.reference == reference)
            .map(|cell| &cell.value)
    })
}

#[test]
fn information_predicates_evaluate_each_array_element_without_coercion() {
    let functions = [
        "ISNUMBER",
        "ISTEXT",
        "ISNONTEXT",
        "ISLOGICAL",
        "ISBLANK",
        "ISERROR",
        "ISERR",
        "ISNA",
    ];
    let formulas: Vec<_> = functions
        .iter()
        .map(|name| format!("{name}({{1,\"1\",TRUE,#N/A,#DIV/0!,\"\"}})"))
        .collect();
    let refs: Vec<_> = formulas.iter().map(String::as_str).collect();
    let result =
        evaluate(&read_xlsx(&workbook(&formula_sheet(&refs), None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    let expected = [
        [true, false, false, false, false, false],
        [false, true, false, false, false, true],
        [true, false, true, true, true, false],
        [false, false, true, false, false, false],
        [false, false, false, false, false, false],
        [false, false, false, true, true, false],
        [false, false, false, false, true, false],
        [false, false, false, true, false, false],
    ];
    for (row, values) in expected.iter().enumerate() {
        for (column, expected) in values.iter().enumerate() {
            let cell = format!("{}{}", (b'A' + column as u8) as char, row + 1);
            assert_eq!(
                computed_or_spilled(&result, &cell),
                Some(&CellValue::Bool(*expected)),
                "{} {cell}",
                functions[row]
            );
        }
    }
    assert_formulas(
        &["SUM(FILTER({1,\"a\",2},ISNUMBER({1,\"a\",2})))"],
        &[CellValue::Number(3.0)],
        false,
    );
}

#[test]
fn information_predicates_preserve_blank_cells_and_empty_strings_in_ranges() {
    let xml = sheet(
        r#"<row r="1"><c r="B1"><v>1</v></c><c r="C1" t="inlineStr"><is><t>1</t></is></c><c r="D1" t="b"><v>1</v></c></row><row r="2"><c r="C2" t="str"><f>""</f><v></v></c><c r="D2" t="e"><v>#N/A</v></c></row><row r="10"><c r="A10"><f>ISBLANK(B1:D2)</f></c></row><row r="14"><c r="A14"><f>ISNUMBER(B1:D2)</f></c></row>"#,
    );
    let result = evaluate(&read_xlsx(&workbook(&xml, None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (cell, expected) in [
        ("A10", false),
        ("B10", false),
        ("C10", false),
        ("A11", true),
        ("B11", false),
        ("C11", false),
        ("A14", true),
        ("B14", false),
        ("C14", false),
        ("A15", false),
        ("B15", false),
        ("C15", false),
    ] {
        assert_eq!(
            computed_or_spilled(&result, cell),
            Some(&CellValue::Bool(expected)),
            "{cell}"
        );
    }
}

#[test]
fn type_recognizes_array_constants_and_calculated_arrays() {
    assert_formulas(
        &[
            "TYPE({1,2;3,4})",
            "TYPE({1})",
            "TYPE(SEQUENCE(2))",
            "TYPE({#N/A,1})",
            "TYPE(1)",
            "TYPE(\"a\")",
            "TYPE(TRUE)",
            "TYPE(NA())",
        ],
        &[
            CellValue::Number(64.0),
            CellValue::Number(64.0),
            CellValue::Number(64.0),
            CellValue::Number(64.0),
            CellValue::Number(1.0),
            CellValue::Number(2.0),
            CellValue::Number(4.0),
            CellValue::Number(16.0),
        ],
        false,
    );
}

#[test]
fn parity_functions_keep_large_numeric_values_and_truncate_fractions() {
    let formulas = [
        "ISODD(1E20)",
        "ISEVEN(1E20)",
        "ISODD(-1E20)",
        "ISEVEN(-1E20)",
        "ISODD(1E100)",
        "ISEVEN(1E100)",
        "ISODD(-3.9)",
        "ISEVEN(-3.9)",
        "ISODD(2.9)",
        "ISEVEN(2.9)",
        "ISODD(0)",
        "ISEVEN(0)",
    ];
    let result =
        evaluate(&read_xlsx(&workbook(&formula_sheet(&formulas), None, false)).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for (index, expected) in [
        false, true, false, true, false, true, true, false, false, true, false, true,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            computed(&result, 0, &format!("A{}", index + 1)),
            Some(&CellValue::Bool(expected)),
            "{}",
            formulas[index]
        );
    }
}

#[test]
fn isformula_rejects_nonreference_arguments_with_value_errors() {
    let bytes = workbook(
        &formula_sheet(&[
            "ISFORMULA(1)",
            "ISFORMULA(\"A1\")",
            "ISFORMULA(1+2)",
            "ISFORMULA(TRUE)",
        ]),
        None,
        false,
    );
    let result = evaluate(&read_xlsx(&bytes).unwrap()).unwrap();
    assert!(result.skipped.is_empty());
    for cell in ["A1", "A2", "A3", "A4"] {
        assert_eq!(
            computed(&result, 0, cell),
            Some(&CellValue::Error("#VALUE!".into()))
        );
    }
}

#[test]
fn held_formula_values_keep_original_formula_identity_and_text() {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><f>FUTUREFUNC( 1 )</f><v>7</v></c><c r="B1"><f>ISFORMULA(A1)</f><v>0</v></c><c r="C1"><f>FORMULATEXT(A1)</f><v>0</v></c><c r="D1"><f>A1+1</f><v>0</v></c></row><row r="2"><c r="A2" t="e"><f>OTHERFUTURE()</f><v>#N/A</v></c><c r="B2"><f>ISFORMULA(A2)</f><v>0</v></c><c r="C2"><f>FORMULATEXT(A2)</f><v>0</v></c></row><row r="3"><c r="A3" t="e"><v>#N/A</v></c><c r="B3"><f>ISFORMULA(A3)</f><v>0</v></c></row></sheetData></worksheet>"#;
    let book = read_xlsx(&workbook(xml, None, false)).unwrap();
    let result = evaluate(&book).unwrap();
    assert_eq!(
        computed(&result, 0, "B1").cloned(),
        Some(CellValue::Bool(true))
    );
    assert_eq!(
        computed(&result, 0, "C1").cloned(),
        Some(CellValue::Text("=FUTUREFUNC( 1 )".into()))
    );
    assert_eq!(
        computed(&result, 0, "D1").cloned(),
        Some(CellValue::Number(8.0))
    );
    assert_eq!(
        computed(&result, 0, "B2").cloned(),
        Some(CellValue::Bool(true))
    );
    assert_eq!(
        computed(&result, 0, "C2").cloned(),
        Some(CellValue::Text("=OTHERFUTURE()".into()))
    );
    assert_eq!(
        computed(&result, 0, "B3").cloned(),
        Some(CellValue::Bool(false))
    );
}

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
        ("F1", "FORMULATEXT(E1)"),
        ("F2", "FORMULATEXT(E2)"),
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
        computed(&outcome, 0, "F1"),
        Some(&CellValue::Text("=SUM(Table1[Qty])".into()))
    );
    assert_eq!(
        computed(&outcome, 0, "F2"),
        Some(&CellValue::Text("=Table1[[#Totals],[Amount]]*2".into()))
    );
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
        computed(&outcome, 0, "F2"),
        Some(&CellValue::Text("=Table1[[#Totals],[Amount]]*2".into()))
    );
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
    assert_eq!(outcome.spills.len(), 1);
    let spill = &outcome.spills[0];
    assert_eq!(spill.anchor, CellRef::parse("A1").unwrap());
    let spilled: Vec<_> = spill
        .cells
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
    assert_eq!(
        spill.range,
        jade_xlsx_reader::CellRange::parse("A1:A3").unwrap()
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
    assert_eq!(outcome.spills.len(), 1);
    let spill = &outcome.spills[0];
    assert_eq!(spill.anchor, CellRef::parse("A1").unwrap());
    let spilled: Vec<_> = spill
        .cells
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
    assert_eq!(
        spill.range,
        jade_xlsx_reader::CellRange::parse("A1:A4").unwrap()
    );
}
