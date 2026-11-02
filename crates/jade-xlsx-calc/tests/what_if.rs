//! What-if probes: trial values in input cells, the formulas reading them
//! evaluated again, the workbook itself untouched.

use std::collections::BTreeMap;
use std::io::Write;

use jade_xlsx_calc::what_if;
use jade_xlsx_reader::{read_xlsx, CellRef, CellValue};
use zip::write::SimpleFileOptions;

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
