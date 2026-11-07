//! CELL reads a cell's format, alignment and protection and its column's
//! width from the file, as Excel does, and CELL("filename") and INFO read
//! where the application has the book saved.

use std::io::Write;

use jade_xlsx_calc::{evaluate, with_document, Document};
use jade_xlsx_reader::{read_xlsx, CellRef, CellValue};
use zip::write::SimpleFileOptions;

fn workbook() -> Vec<u8> {
    let styles = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="&quot;$&quot;#,##0.00_);[Red]\(&quot;$&quot;#,##0.00\)"/></numFmts><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="1"><fill><patternFill patternType="none"/></fill></fills><borders count="1"><border/></borders><cellXfs count="4"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/><xf numFmtId="164" fontId="0" fillId="0" borderId="0" applyNumberFormat="1"/><xf numFmtId="10" fontId="0" fillId="0" borderId="0" applyNumberFormat="1" applyProtection="1"><protection locked="0"/></xf><xf numFmtId="0" fontId="0" fillId="0" borderId="0" applyAlignment="1"><alignment horizontal="center"/></xf></cellXfs></styleSheet>"#;
    let sheet = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="2" max="2" width="20.7109375" customWidth="1"/></cols><sheetData><row r="1"><c r="A1" s="1"><v>5</v></c><c r="B1" s="2"><v>0.5</v></c><c r="C1" s="3" t="inlineStr"><is><t>mid</t></is></c></row><row r="2"><c r="A2" t="str"><f>CELL("format",A1)</f><v></v></c><c r="B2" t="str"><f>CELL("format",B1)</f><v></v></c><c r="C2" t="str"><f>CELL("prefix",C1)</f><v></v></c><c r="D2"><f>CELL("protect",B1)</f><v></v></c><c r="E2"><f>CELL("color",A1)</f><v></v></c></row><row r="3"><c r="A3"><f>INDEX(CELL("width",B1),1)</f><v></v></c><c r="B3"><f>INDEX(CELL("width",A1),1)</f><v></v></c><c r="C3" t="str"><f>CELL("filename",A1)</f><v></v></c><c r="D3" t="str"><f>INFO("directory")</f><v></v></c></row></sheetData></worksheet>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let root = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", root),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", rels),
            ("xl/worksheets/sheet1.xml", sheet),
            ("xl/styles.xml", styles),
        ] {
            writer
                .start_file(name, SimpleFileOptions::default())
                .expect("part");
            writer.write_all(contents.as_bytes()).expect("write");
        }
        writer.finish().expect("archive");
    }
    buffer
}

#[test]
fn formulatext_respects_hidden_formulas_only_while_the_source_sheet_is_protected() {
    let mut book = read_xlsx(&workbook()).unwrap();
    book.sheets[0].rows.retain(|row| row.index < 2);
    for (reference, formula) in [
        ("A1", "1+2"),
        ("B1", "2+3"),
        ("A2", "FORMULATEXT(A1)"),
        ("B2", "FORMULATEXT(B1)"),
        ("C2", "ISFORMULA(A1)"),
        ("D2", "FORMULATEXT(A1:B1)"),
        ("E2", "FORMULATEXT (A1)"),
    ] {
        let reference = CellRef::parse(reference).unwrap();
        let cell = book.sheets[0]
            .rows
            .iter_mut()
            .flat_map(|row| &mut row.cells)
            .find(|cell| cell.reference == reference)
            .unwrap();
        cell.formula = Some(formula.into());
    }
    for (protected, hidden) in [(false, true), (true, true), (true, false), (false, true)] {
        book.sheets[0].protection = protected.then(Default::default);
        book.styles.cell_formats[1].formula_hidden = hidden;
        let result = evaluate(&book).unwrap();
        assert!(result.skipped.is_empty());
        for reference in ["A2", "D2", "E2"] {
            let expected = if protected && hidden {
                CellValue::Error("#N/A".into())
            } else {
                CellValue::Text("=1+2".into())
            };
            let at = CellRef::parse(reference).unwrap();
            assert_eq!(
                result
                    .cells
                    .iter()
                    .find(|cell| cell.reference == at)
                    .map(|cell| &cell.value),
                Some(&expected),
                "{reference}, protected={protected}, hidden={hidden}"
            );
        }
        for (reference, expected) in [
            ("B2", CellValue::Text("=2+3".into())),
            ("C2", CellValue::Bool(true)),
        ] {
            let at = CellRef::parse(reference).unwrap();
            assert_eq!(
                result
                    .cells
                    .iter()
                    .find(|cell| cell.reference == at)
                    .map(|cell| &cell.value),
                Some(&expected)
            );
        }
    }
}

#[test]
fn named_formula_queries_cannot_bypass_hidden_formula_protection() {
    let mut book = read_xlsx(&workbook()).unwrap();
    book.sheets[0].rows.retain(|row| row.index < 2);
    for cell in book.sheets[0]
        .rows
        .iter_mut()
        .flat_map(|row| &mut row.cells)
    {
        cell.formula = match cell.reference {
            at if at == CellRef::parse("A1").unwrap() => Some("1+2".into()),
            at if at == CellRef::parse("E2").unwrap() => Some("HiddenQuery".into()),
            _ => None,
        };
    }
    book.styles.cell_formats[1].formula_hidden = true;
    book.defined_names
        .push(jade_xlsx_reader::model::DefinedName {
            name: "HiddenQuery".into(),
            formula: "FORMULATEXT(Sheet1!$A$1)".into(),
            local_sheet_index: None,
            hidden: false,
        });
    for protected in [false, true, false] {
        book.sheets[0].protection = protected.then(Default::default);
        let result = evaluate(&book).unwrap();
        assert!(result.skipped.is_empty());
        let at = CellRef::parse("E2").unwrap();
        let expected = if protected {
            CellValue::Error("#N/A".into())
        } else {
            CellValue::Text("=1+2".into())
        };
        assert_eq!(
            result
                .cells
                .iter()
                .find(|cell| cell.reference == at)
                .map(|cell| &cell.value),
            Some(&expected)
        );
    }
}

#[test]
fn cell_reads_formats_and_widths_from_the_file() {
    let workbook = read_xlsx(&workbook()).expect("read");
    let document = Document {
        file_name: Some("Budget.xlsx".into()),
        directory: Some("/Users/me/Documents/".into()),
        ..Default::default()
    };
    let outcome = with_document(document, || evaluate(&workbook)).expect("evaluate");
    let computed = |reference: &str| {
        let reference = CellRef::parse(reference).expect("reference");
        outcome
            .cells
            .iter()
            .find(|cell| cell.reference == reference)
            .map(|cell| cell.value.clone())
    };
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    assert_eq!(computed("A2"), Some(CellValue::Text("C2-".into())));
    assert_eq!(computed("B2"), Some(CellValue::Text("P2".into())));
    assert_eq!(computed("C2"), Some(CellValue::Text("^".into())));
    assert_eq!(computed("D2"), Some(CellValue::Number(0.0)));
    assert_eq!(computed("E2"), Some(CellValue::Number(1.0)));
    // 20 characters as set, 8 by default (8.43).
    assert_eq!(computed("A3"), Some(CellValue::Number(20.0)));
    assert_eq!(computed("B3"), Some(CellValue::Number(8.0)));
    assert_eq!(
        computed("C3"),
        Some(CellValue::Text(
            "/Users/me/Documents/[Budget.xlsx]Sheet1".into()
        ))
    );
    assert_eq!(
        computed("D3"),
        Some(CellValue::Text("/Users/me/Documents/".into()))
    );
}
