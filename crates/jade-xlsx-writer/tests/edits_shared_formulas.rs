mod common;

use common::workbook_with_sheet;
use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, CellRef};
use jade_xlsx_writer::{write_xlsx, CellEdit, NewValue, SheetEdits};

const SHEET: &str = "xl/worksheets/sheet1.xml";
const XML: &str = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1"><v>2</v></c><c r="B1"><f t="shared" ref="B1:B3" si="4">A1*2</f><v>4</v></c><c r="C1"><f t="shared" ref="C1:C2" si="5">A1+1</f><v>3</v></c></row>
<row r="2"><c r="A2"><v>3</v></c><c r="B2"><f t="shared" si="4"/><v>6</v></c><c r="C2"><f t="shared" si="5"/><v>4</v></c></row>
<row r="3"><c r="A3"><v>4</v></c><c r="B3"><f t="shared" si="4"/><v>8</v></c></row>
</sheetData></worksheet>"#;

fn write(reference: &str, value: NewValue) -> (Vec<u8>, String) {
    let bytes = write_xlsx(
        &workbook_with_sheet(XML),
        &[SheetEdits {
            part_name: SHEET.to_string(),
            cells: vec![CellEdit {
                reference: CellRef::parse(reference).expect("reference"),
                value,
                style_index: None,
            }],
        }],
    )
    .expect("write");
    let xml = OpcPackage::open(&bytes)
        .expect("package")
        .required_part_str(SHEET)
        .expect("sheet");
    (bytes, xml)
}

#[test]
fn replacing_a_shared_formula_master_preserves_follower_formulas_and_caches() {
    let (bytes, xml) = write("B1", NewValue::Number(99.0));
    assert!(!xml.contains("si=\"4\""), "{xml}");
    assert!(xml.contains("<f t=\"shared\" ref=\"C1:C2\" si=\"5\">A1+1</f>"));
    assert!(xml.contains("<c r=\"B2\"><f>A2*2</f><v>6</v></c>"), "{xml}");
    assert!(xml.contains("<c r=\"B3\"><f>A3*2</f><v>8</v></c>"), "{xml}");

    let workbook = read_xlsx(&bytes).expect("read");
    let cells = &workbook.sheets[0].rows;
    assert_eq!(cells[0].cells[1].formula, None);
    assert_eq!(cells[1].cells[1].formula.as_deref(), Some("A2*2"));
    assert_eq!(cells[2].cells[1].formula.as_deref(), Some("A3*2"));
}

#[test]
fn replacing_a_shared_formula_follower_detaches_the_entire_group() {
    let (bytes, xml) = write("B2", NewValue::Text("manual".to_string()));
    assert!(!xml.contains("si=\"4\""), "{xml}");
    assert!(xml.contains("<f t=\"shared\" ref=\"C1:C2\" si=\"5\">A1+1</f>"));
    assert!(xml.contains("<c r=\"B1\"><f>A1*2</f><v>4</v></c>"), "{xml}");
    assert!(xml.contains("<c r=\"B3\"><f>A3*2</f><v>8</v></c>"), "{xml}");

    let workbook = read_xlsx(&bytes).expect("read");
    let cells = &workbook.sheets[0].rows;
    assert_eq!(cells[0].cells[1].formula.as_deref(), Some("A1*2"));
    assert_eq!(cells[1].cells[1].formula, None);
    assert_eq!(cells[2].cells[1].formula.as_deref(), Some("A3*2"));
}

#[test]
fn restyling_a_shared_formula_does_not_rewrite_its_group() {
    let (_, xml) = write("B2", NewValue::Unchanged);
    assert!(xml.contains("<f t=\"shared\" ref=\"B1:B3\" si=\"4\">A1*2</f>"));
    assert!(xml.contains("<f t=\"shared\" si=\"4\"/>"));
}
