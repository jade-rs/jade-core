//! Opaque worksheet extensions retain their original source during grid edits.
mod common;

use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{CellRange, CellRef};

#[test]
fn foreign_extension_payloads_stay_exact_through_grid_history_and_reopen() {
    for main in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        for prefix in ["", "s:"] {
            for payload in [
                r#"<u:mergeCells count="1"><u:mergeCell ref="D4:E5" keep="exact"/></u:mergeCells>"#.to_string(),
                r#"<u:rowBreaks count="1" manualBreakCount="1"><u:brk id="4" min="0" max="16383" man="1"/></u:rowBreaks>"#.into(),
                r#"<u:conditionalFormatting sqref="D4:E5"><u:cfRule type="expression"><u:formula>D4&gt;0</u:formula></u:cfRule></u:conditionalFormatting>"#.into(),
                r#"<u:cols><u:col min="4" max="5" width="20" customWidth="1"/></u:cols>"#.into(),
                r#"<u:hyperlinks><u:hyperlink ref="D4:E5" location="D4"/></u:hyperlinks>"#.into(),
                r#"<u:dimension ref="D4:E5" quoted="&quot;"/>"#.into(),
                format!(r#"<u:payload xmlns="{main}"><mergeCells count="1"><mergeCell ref="D4:E5"/></mergeCells></u:payload>"#),
                format!(r#"<u:payload xmlns:q="{main}" xmlns:s="urn:shadow" xmlns:x="urn:shadow-x"><q:mergeCells count="1"><q:mergeCell ref="D4:E5"/></q:mergeCells><![CDATA[保持 A1]]><!--exact--></u:payload>"#),
            ] {
                for rows in [true, false] {
                    let sheet = format!(r#"<{prefix}worksheet xmlns="{main}" xmlns:s="{main}" xmlns:u="urn:unknown-extension" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main"><{prefix}sheetData><{prefix}row r="1"><{prefix}c r="A1"><{prefix}v>1</{prefix}v></{prefix}c><{prefix}c r="B1"><{prefix}f>A1+1</{prefix}f><{prefix}v>2</{prefix}v></{prefix}c></{prefix}row></{prefix}sheetData><{prefix}mergeCells count="1"><{prefix}mergeCell ref="D4:E5"/></{prefix}mergeCells><{prefix}extLst><{prefix}ext uri="urn:unknown-extension">{payload}</{prefix}ext><{prefix}ext uri="{{CCE6A557-97BC-4B89-ADB6-D9C93CAAB3DF}}"><x:dataValidations count="1"><x:dataValidation type="custom"><x:formula1><m:f>$A1&gt;0</m:f></x:formula1><m:sqref>H3:H5</m:sqref></x:dataValidation></x:dataValidations></{prefix}ext></{prefix}extLst></{prefix}worksheet>"#);
                    let mut book = HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet)).unwrap();
                    let before = book.bytes().to_vec();
                    let (input, formula, merged, validation, bound) = if rows {
                        book.insert_rows("Sheet1", 1, 1).unwrap();
                        ("A2", "B2", "D5:E6", "H4:H6", "$A2>0")
                    } else {
                        book.insert_columns("Sheet1", "A", 1).unwrap();
                        ("B1", "C1", "E4:F5", "I3:I5", "$B1>0")
                    };
                    assert_eq!(book.cell("Sheet1", formula).unwrap().display, "2");
                    assert_eq!(book.workbook().sheets[0].merges, [CellRange::parse(merged).unwrap()]);
                    let rule = &book.workbook().sheets[0].data_validations[0];
                    assert_eq!(rule.ranges, [CellRange::parse(validation).unwrap()]);
                    assert_eq!(rule.formula1.as_deref(), Some(bound));
                    let after = book.bytes().to_vec();
                    book.undo().unwrap();
                    assert_eq!(book.bytes(), before);
                    book.redo().unwrap();
                    assert_eq!(book.bytes(), after);
                    let saved = book.save_bytes().unwrap();
                    assert_payload(&saved, &payload);
                    let mut reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
                    assert_eq!(reopened.cell("Sheet1", formula).unwrap().display, "2");
                    reopened.set("Sheet1", input, jade_xlsx_headless_api::CellInput::Number(5.0)).unwrap();
                    assert_eq!(reopened.cell("Sheet1", formula).unwrap().display, "6");
                    assert_eq!(reopened.workbook().sheets[0].merges[0].start, CellRef::parse(if rows { "D5" } else { "E4" }).unwrap());
                    assert_payload(&reopened.save_bytes().unwrap(), &payload);
                }
            }
        }
    }
}

fn assert_payload(bytes: &[u8], payload: &str) {
    let package = OpcPackage::open(bytes).unwrap();
    let sheet = package
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap();
    assert!(
        sheet.contains(payload),
        "unprocessed extension was changed: {sheet}"
    );
}

#[test]
fn unknown_extension_uri_keeps_standard_named_payloads_opaque() {
    for namespace in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://schemas.microsoft.com/office/excel/2006/main",
        "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main",
    ] {
        let payload = format!(
            r#"<q:mergeCells xmlns:q="{namespace}" count="1"><q:mergeCell ref="D4:E5"/></q:mergeCells>"#
        );
        let sheet = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/><mergeCells count="1"><mergeCell ref="D4:E5"/></mergeCells><extLst><ext uri="urn:unknown-extension">{payload}</ext></extLst></worksheet>"#
        );
        let mut book =
            HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet)).unwrap();
        book.insert_rows("Sheet1", 1, 1).unwrap();
        assert_eq!(
            book.workbook().sheets[0].merges,
            [CellRange::parse("D5:E6").unwrap()]
        );
        assert_payload(&book.save_bytes().unwrap(), &payload);
    }
}
