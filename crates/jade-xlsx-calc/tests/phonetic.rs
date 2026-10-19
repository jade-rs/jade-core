//! PHONETIC reads the furigana Excel saved with the text (`<rPh>` in the
//! shared strings), as Japanese Excel does: 山田太郎 typed with ヤマダ over 山田
//! and タロウ over 太郎 reads ヤマダタロウ. A cell without furigana gives its
//! text, and a range gives its cells' readings joined.

use std::io::Write;

use jade_xlsx_calc::evaluate;
use jade_xlsx_reader::{read_xlsx, CellRef, CellValue};
use zip::write::SimpleFileOptions;

fn workbook() -> Vec<u8> {
    let shared = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2"><si><t>山田太郎</t><rPh sb="0" eb="2"><t>ヤマダ</t></rPh><rPh sb="2" eb="4"><t>タロウ</t></rPh><phoneticPr fontId="1"/></si><si><t>佐藤</t><rPh sb="0" eb="2"><t>サトウ</t></rPh><phoneticPr fontId="1"/></si></sst>"#;
    let sheet = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="str"><f>PHONETIC(A1)</f><v></v></c></row><row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2" t="str"><f>PHONETIC(A1:A3)</f><v></v></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>abc</t></is></c><c r="B3" t="str"><f>PHONETIC(A3)</f><v></v></c></row></sheetData></worksheet>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="名簿" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#;
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
            ("xl/sharedStrings.xml", shared),
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
fn phonetic_reads_the_saved_furigana() {
    let workbook = read_xlsx(&workbook()).expect("read");
    // The cell's value is the text, the furigana kept beside it.
    assert_eq!(
        workbook.sheets[0].rows[0].cells[0].value,
        CellValue::Text("山田太郎".into())
    );
    let outcome = evaluate(&workbook).expect("evaluate");
    let computed = |reference: &str| {
        let reference = CellRef::parse(reference).expect("reference");
        outcome
            .cells
            .iter()
            .find(|cell| cell.reference == reference)
            .map(|cell| cell.value.clone())
    };
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    assert_eq!(computed("B1"), Some(CellValue::Text("ヤマダタロウ".into())));
    assert_eq!(
        computed("B2"),
        Some(CellValue::Text("ヤマダタロウサトウabc".into()))
    );
    assert_eq!(computed("B3"), Some(CellValue::Text("abc".into())));
}
