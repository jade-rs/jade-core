//! What-if probes: trial values in input cells, the formulas reading them
//! evaluated again, the workbook itself untouched.

use std::collections::BTreeMap;
use std::io::Write;

use jade_xlsx_calc::what_if;
use jade_xlsx_reader::{read_xlsx, CellRef, CellValue};
use zip::write::SimpleFileOptions;

#[test]
fn cse_member_information_restores_after_trial_literals() {
    let book = read_xlsx(&package(r#"<row r="1"><c r="A1"><f t="array" ref="A1:A3">SEQUENCE(3, 1)</f><v>1</v></c><c r="C1"><f>FORMULATEXT(A2)</f></c><c r="D1"><f>ISFORMULA(A2)</f></c></row><row r="2"><c r="A2"><v>2</v></c></row><row r="3"><c r="A3"><v>3</v></c></row>"#)).unwrap();
    let mut probe = what_if(&book, "UTC", &BTreeMap::new()).unwrap();
    let output = [(0, CellRef::new(2, 0)), (0, CellRef::new(3, 0))];
    let original = vec![
        CellValue::Text("{=SEQUENCE(3, 1)}".into()),
        CellValue::Bool(true),
    ];
    assert_eq!(probe.probe(&book, &[], &output).unwrap(), original);
    for at in [CellRef::new(0, 1), CellRef::new(0, 0)] {
        assert_eq!(
            probe
                .probe(&book, &[(0, at, CellValue::Number(99.0))], &output)
                .unwrap(),
            vec![CellValue::Error("#N/A".into()), CellValue::Bool(false)]
        );
        assert_eq!(probe.probe(&book, &[], &output).unwrap(), original);
    }
}

#[test]
fn legacy_fixed_array_bounds_survive_trial_input_growth_and_restoration() {
    let book = read_xlsx(&package(r#"<row r="1"><c r="A1"><f t="array" ref="A1:A3">_xlfn.SEQUENCE(C1)</f><v>1</v></c><c r="C1"><v>2</v></c></row><row r="2"><c r="A2"><v>2</v></c></row><row r="3"><c r="A3" t="e"><v>#N/A</v></c></row>"#)).unwrap();
    let mut probe = what_if(&book, "UTC", &BTreeMap::new()).unwrap();
    let output = [(0, CellRef::new(0, 2)), (0, CellRef::new(0, 3))];
    let original = vec![CellValue::Error("#N/A".into()), CellValue::Empty];
    assert_eq!(probe.probe(&book, &[], &output).unwrap(), original);
    assert_eq!(
        probe
            .probe(
                &book,
                &[(0, CellRef::new(2, 0), CellValue::Number(5.0))],
                &output
            )
            .unwrap(),
        vec![CellValue::Number(3.0), CellValue::Empty]
    );
    assert_eq!(probe.probe(&book, &[], &output).unwrap(), original);
}

#[test]
fn legacy_cse_formula_identity_survives_trial_replacement() {
    let book=read_xlsx(&package(r#"<row r="1"><c r="A1"><f t="array" ref="A1">SUM(1, 2)</f><v>3</v></c><c r="B1"><f>FORMULATEXT(A1)</f></c></row>"#)).unwrap();
    let mut probe = what_if(&book, "UTC", &BTreeMap::new()).unwrap();
    let output = [(0, CellRef::new(1, 0))];
    let text = vec![CellValue::Text("{=SUM(1, 2)}".into())];
    assert_eq!(probe.probe(&book, &[], &output).unwrap(), text);
    assert_eq!(
        probe
            .probe(
                &book,
                &[(0, CellRef::new(0, 0), CellValue::Number(10.0))],
                &output
            )
            .unwrap(),
        vec![CellValue::Error("#N/A".into())]
    );
    assert_eq!(probe.probe(&book, &[], &output).unwrap(), text);
}

#[test]
fn parsed_source_formula_text_is_the_same_before_and_after_trial_replacement() {
    let book=read_xlsx(&package(r#"<row r="1"><c r="A1"><f>SUM(1, 2)</f><v>3</v></c><c r="B1"><f>FORMULATEXT(A1)</f></c></row>"#)).unwrap();
    let mut probe = what_if(&book, "UTC", &BTreeMap::new()).unwrap();
    let source = (0, CellRef::new(0, 0));
    let output = [(0, CellRef::new(1, 0))];
    let text = vec![CellValue::Text("=SUM(1, 2)".into())];
    assert_eq!(probe.probe(&book, &[], &output).unwrap(), text);
    assert_eq!(
        probe
            .probe(
                &book,
                &[(source.0, source.1, CellValue::Number(10.0))],
                &output
            )
            .unwrap(),
        vec![CellValue::Error("#N/A".into())]
    );
    assert_eq!(probe.probe(&book, &[], &output).unwrap(), text);
}

fn package(rows: &str) -> Vec<u8> {
    let parts = [
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_string(),
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#.to_string(),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.to_string(),
        ),
        (
            "xl/worksheets/sheet1.xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#
            ),
        ),
    ];
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in parts {
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }
    buffer
}

#[test]
fn probes_keep_error_looking_text_distinct_from_error_values() {
    let book = read_xlsx(&package(r##"<row r="1"><c r="A1"><v>1</v></c><c r="B1" t="str"><f>IF(A1=1,"#N/A",NA())</f><v>#N/A</v></c><c r="C1" t="str"><f>VALUETOTEXT(B1,1)</f><v>#N/A</v></c></row>"##)).unwrap();
    let mut probe = what_if(&book, "UTC", &BTreeMap::new()).unwrap();
    let outputs = [(0, CellRef::new(1, 0)), (0, CellRef::new(2, 0))];
    let original = vec![
        CellValue::Text("#N/A".into()),
        CellValue::Text("\"#N/A\"".into()),
    ];
    assert_eq!(probe.probe(&book, &[], &outputs).unwrap(), original);
    assert_eq!(
        probe
            .probe(
                &book,
                &[(0, CellRef::new(0, 0), CellValue::Number(0.0))],
                &outputs
            )
            .unwrap(),
        vec![
            CellValue::Error("#N/A".into()),
            CellValue::Text("#N/A".into())
        ]
    );
    assert_eq!(probe.probe(&book, &[], &outputs).unwrap(), original);
}

#[test]
fn held_formula_metadata_and_cached_values_restore_after_trial_constants() {
    let book = read_xlsx(&package(r#"<row r="1"><c r="A1"><f>FUTUREFUNC()</f><v>7</v></c><c r="B1"><f>ISFORMULA(A1)</f><v>0</v></c><c r="C1"><f>FORMULATEXT(A1)</f><v>0</v></c><c r="D1"><f>A1+1</f><v>0</v></c></row>"#)).unwrap();
    let mut probe = what_if(&book, "UTC", &BTreeMap::new()).unwrap();
    let outputs = [
        (0, CellRef::new(1, 0)),
        (0, CellRef::new(2, 0)),
        (0, CellRef::new(3, 0)),
    ];
    let original = vec![
        CellValue::Bool(true),
        CellValue::Text("=FUTUREFUNC()".into()),
        CellValue::Number(8.0),
    ];
    assert_eq!(probe.probe(&book, &[], &outputs).unwrap(), original);
    assert_eq!(
        probe
            .probe(
                &book,
                &[(0, CellRef::new(0, 0), CellValue::Number(2.0))],
                &outputs
            )
            .unwrap(),
        vec![
            CellValue::Bool(false),
            CellValue::Error("#N/A".into()),
            CellValue::Number(3.0)
        ]
    );
    assert_eq!(probe.probe(&book, &[], &outputs).unwrap(), original);
    probe
        .probe(
            &book,
            &[(0, CellRef::new(0, 0), CellValue::Number(2.0))],
            &outputs,
        )
        .unwrap();
    assert_eq!(
        probe
            .formula_holds(&book, 0, CellRef::new(3, 0), "D1=8", &[CellRef::new(3, 0)])
            .unwrap(),
        Some(vec![true])
    );
}

#[test]
fn a_probe_evaluates_with_trial_values_and_then_puts_them_back() {
    let book = read_xlsx(&package(
        r#"<row r="1"><c r="A1"><v>2</v></c><c r="B1"><v>3</v></c><c r="C1"><f>A1*B1</f><v>6</v></c></row>"#,
    ))
    .unwrap();
    let mut probe = what_if(&book, "UTC", &BTreeMap::new()).unwrap();
    let (a1, b1, c1) = (
        (0, CellRef::new(0, 0)),
        (0, CellRef::new(1, 0)),
        (0, CellRef::new(2, 0)),
    );
    assert_eq!(probe.number(&book, a1, 10.0, c1).unwrap(), Some(30.0));
    // B1 tried now, and A1 holds its 2 again.
    assert_eq!(probe.number(&book, b1, 5.0, c1).unwrap(), Some(10.0));
    assert_eq!(
        probe.probe(&book, &[], &[c1]).unwrap(),
        vec![CellValue::Number(6.0)]
    );
    // A formula cell tried as a value, then its formula again.
    assert_eq!(
        probe
            .probe(&book, &[(0, c1.1, CellValue::Number(1.0))], &[c1])
            .unwrap(),
        vec![CellValue::Number(1.0)]
    );
    assert_eq!(
        probe.probe(&book, &[], &[c1]).unwrap(),
        vec![CellValue::Number(6.0)]
    );
}

#[test]
fn a_computed_criterion_is_decided_for_each_record_from_the_first() {
    // Sales in A2:A5; the criterion in C2, written for the first record:
    // above the average of them all.
    let book = read_xlsx(&package(
        r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Sales</t></is></c></row><row r="2"><c r="A2"><v>10</v></c><c r="C2"><f>A2&gt;AVERAGE($A$2:$A$5)</f><v>0</v></c></row><row r="3"><c r="A3"><v>40</v></c></row><row r="4"><c r="A4"><v>20</v></c></row><row r="5"><c r="A5"><v>50</v></c></row>"#,
    ))
    .unwrap();
    let mut probe = what_if(&book, "UTC", &BTreeMap::new()).unwrap();
    let cells: Vec<CellRef> = (1..=4).map(|row| CellRef::new(2, row)).collect();
    let criterion = (0, CellRef::new(2, 1));
    assert_eq!(
        probe
            .formula_holds(
                &book,
                criterion.0,
                criterion.1,
                "A2>AVERAGE($A$2:$A$5)",
                &cells
            )
            .unwrap(),
        Some(vec![false, true, false, true])
    );
    // A formula the engine cannot read decides nothing.
    assert_eq!(
        probe
            .formula_holds(&book, criterion.0, criterion.1, "A2>(", &cells)
            .unwrap(),
        None
    );
}
