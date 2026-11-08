mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{data_validations::parse_data_validations, CellRange};
#[test]
fn rewriting_source_text_preserves_attributes_comments_and_opaque_children() {
    check(false);
}
#[test]
fn processed_text_sources_are_read_and_edited_with_inherited_namespace_scope() {
    check(true);
}
fn check(processed: bool) {
    for ns in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        for extended in [false, true] {
            let shadow = "<u:pass>A99</u:pass>";
            let body = if processed {
                "<v:pass>A2</v:pass>"
            } else {
                "A2"
            };
            let attrs = "u:note=\"keep\" xmlns:u=\"urn:two\" mc:Ignorable=\"u\"";
            let source = format!("<!--first-->{body}{shadow}<!--last-->");
            let rule = if extended {
                format!("<x:dataValidations count=\"1\"><x:dataValidation type=\"whole\" operator=\"between\"><x:formula1><m:f {attrs}>{source}</m:f></x:formula1><x:formula2><m:f u:note=\"second\"><!--second-->10</m:f></x:formula2><m:sqref u:note=\"coverage\"><!--range-->B2</m:sqref></x:dataValidation></x:dataValidations>")
            } else {
                format!("<dataValidations count=\"1\"><dataValidation type=\"whole\" operator=\"between\" sqref=\"B2\"><formula1 {attrs}>{source}</formula1><formula2 u:note=\"second\"><!--second-->10</formula2></dataValidation></dataValidations>")
            };
            let tail = if extended {
                format!("<extLst><ext uri=\"{{CCE6A557-97BC-4B89-ADB6-D9C93CAAB3DF}}\">{rule}</ext></extLst>")
            } else {
                rule
            };
            let original = format!(
                r#"<worksheet xmlns="{ns}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:one" xmlns:v="urn:one" mc:Ignorable="u" mc:ProcessContent="u:pass"><sheetData/>{tail}</worksheet>"#
            );
            let mut rule = parse_data_validations(&original).unwrap().remove(0);
            assert_eq!(rule.formula1.as_deref(), Some("A2"));
            rule.formula1 = Some("B3".into());
            rule.formula2 = Some("20".into());
            rule.ranges = vec![CellRange::parse("C4").unwrap()];
            let mut book =
                HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&original))
                    .unwrap();
            let before = book.bytes().to_vec();
            book.set_data_validation("Sheet1", Some(0), &rule).unwrap();
            let after = book.bytes().to_vec();
            book.undo().unwrap();
            assert_eq!(book.bytes(), before);
            book.redo().unwrap();
            assert_eq!(book.bytes(), after);
            let bytes = book.save_bytes().unwrap();
            let saved = OpcPackage::open(&bytes)
                .unwrap()
                .required_part_str("xl/worksheets/sheet1.xml")
                .unwrap();
            for marker in [
                "u:note=\"keep\"",
                "u:note=\"second\"",
                "<!--first-->",
                "<!--last-->",
                "<!--second-->",
                shadow,
            ] {
                assert!(saved.contains(marker), "{marker}: {saved}");
            }
            if extended {
                assert!(saved.contains("u:note=\"coverage\""), "{saved}");
                assert!(saved.contains("<!--range-->C4"), "{saved}");
            }
            if processed {
                assert!(saved.contains("<v:pass>B3</v:pass>"), "{saved}");
            }
            assert_eq!(parse_data_validations(&saved).unwrap(), vec![rule.clone()]);
            assert!(!saved.contains("_jade_ctx"), "{saved}");
            let mut reopened = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
            let again = OpcPackage::open(&reopened.save_bytes().unwrap())
                .unwrap()
                .required_part_str("xl/worksheets/sheet1.xml")
                .unwrap();
            assert_eq!(again, saved);
            reopened.insert_rows("Sheet1", 2, 1).unwrap();
            reopened.insert_columns("Sheet1", "B", 1).unwrap();
            let shifted = OpcPackage::open(&reopened.save_bytes().unwrap())
                .unwrap()
                .required_part_str("xl/worksheets/sheet1.xml")
                .unwrap();
            let mut expected = rule.clone();
            expected.formula1 = Some("C4".into());
            expected.ranges = vec![CellRange::parse("D5").unwrap()];
            assert_eq!(
                parse_data_validations(&shifted).unwrap(),
                vec![expected],
                "{shifted}"
            );
            assert!(shifted.contains(shadow), "{shifted}");
        }
    }
}
