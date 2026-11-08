mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{Axis, CellRange, Hyperlink};

#[test]
fn replacement_keeps_live_metadata_and_cleared_metadata_stays_inactive() {
    for namespace in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        let metadata = "<!--keep--><u:hyperlink ref=\"B2\"/><u:payload>B2</u:payload>";
        let xml = format!(
            r#"<worksheet xmlns="{namespace}" xmlns:u="urn:foreign" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="u"><sheetData/><hyperlinks u:flag="keep"><hyperlink ref="B2" location="Sheet1!A1"/>{metadata}</hyperlinks></worksheet>"#
        );
        let mut book =
            HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&xml)).unwrap();
        let link = Hyperlink {
            reference: CellRange::parse("B2").unwrap(),
            target: None,
            location: Some("Destination".into()),
            display: None,
            tooltip: None,
        };
        book.set_hyperlink("Sheet1", &link).unwrap();
        let saved = book.save_bytes().unwrap();
        let result = OpcPackage::open(&saved)
            .unwrap()
            .required_part_str("xl/worksheets/sheet1.xml")
            .unwrap();
        assert!(result.contains(metadata));
        assert!(result.contains("<hyperlinks u:flag=\"keep\">"), "{result}");
        assert!(!result.contains("urn:jade-office:preserved-ooxml"));
        book.clear_hyperlinks("Sheet1", &[link.reference]).unwrap();
        book.set_hyperlink("Sheet1", &link).unwrap();
        book.insert_rows("Sheet1", 1, 1).unwrap();
        let saved = book.save_bytes().unwrap();
        let result = OpcPackage::open(&saved)
            .unwrap()
            .required_part_str("xl/worksheets/sheet1.xml")
            .unwrap();
        assert!(result.contains(metadata), "{result}");
        let reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
        let links = &reopened.workbook().sheets[0].hyperlinks;
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].reference, CellRange::parse("B3").unwrap());
        assert_eq!(links[0].location.as_deref(), Some("Destination"));
    }
}

#[test]
fn unrelated_edits_leave_preexisting_empty_containers_untouched() {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/><hyperlinks/></worksheet>"#;
    let mut book = HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(xml)).unwrap();
    assert!(!book
        .clear_hyperlinks("Sheet1", &[CellRange::parse("B2").unwrap()])
        .unwrap());
    book.insert_rows("Sheet1", 1, 1).unwrap();
    let result = OpcPackage::open(&book.save_bytes().unwrap())
        .unwrap()
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap();
    assert!(result.contains("<hyperlinks/>"), "{result}");
    assert!(!result.contains("urn:jade-office:preserved-ooxml"));
}

#[test]
fn removing_last_link_preserves_metadata_without_an_active_empty_list() {
    let mut failures = Vec::new();
    for strict in [false, true] {
        let namespace = if strict {
            "http://purl.oclc.org/ooxml/spreadsheetml/main"
        } else {
            "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
        };
        for opaque in [false, true] {
            for mode in ["clear", "partial-row", "partial-column", "row", "column"] {
                let attributes = if opaque { " u:flag=\"keep\"" } else { "" };
                let metadata = if opaque {
                    "<!--keep--><u:hyperlink ref=\"B2\"/><u:payload>B2</u:payload>"
                } else {
                    ""
                };
                let xml = format!(
                    r#"<worksheet xmlns="{namespace}" xmlns:u="urn:foreign" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="u"><sheetData/><hyperlinks{attributes}><hyperlink ref="B2" location="Sheet1!A1"/>{metadata}</hyperlinks></worksheet>"#
                );
                let mut book =
                    HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&xml)).unwrap();
                let before = book.bytes().to_vec();
                match mode {
                    "clear" => {
                        book.clear_hyperlinks("Sheet1", &[CellRange::parse("B2").unwrap()])
                            .unwrap();
                    }
                    "partial-row" | "partial-column" => {
                        book.shift_cells(
                            "Sheet1",
                            CellRange::parse("B2").unwrap(),
                            if mode == "partial-row" {
                                Axis::Rows
                            } else {
                                Axis::Columns
                            },
                            false,
                        )
                        .unwrap();
                    }
                    "row" => {
                        book.delete_rows("Sheet1", 2, 1).unwrap();
                    }
                    _ => {
                        book.delete_columns("Sheet1", "B", 1).unwrap();
                    }
                }
                let after = book.bytes().to_vec();
                book.undo().unwrap();
                assert_eq!(book.bytes(), before);
                book.redo().unwrap();
                assert_eq!(book.bytes(), after);
                let saved = book.save_bytes().unwrap();
                if let Some(directory) = std::env::var_os("JADE_SCHEMA_AUDIT_DIR") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    std::fs::write(
                        directory.join(format!("hyperlink-{strict}-{opaque}-{mode}.xlsx")),
                        &saved,
                    )
                    .unwrap();
                }
                let result = OpcPackage::open(&saved)
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                let archived = result.contains("urn:jade-office:preserved-ooxml");
                if opaque
                    && (!result.contains(metadata)
                        || !result.contains("u:flag=\"keep\"")
                        || !archived)
                {
                    failures.push(format!(
                        "metadata lost or active: {strict}/{mode}: {result}"
                    ));
                }
                if !opaque && result.contains("<hyperlinks") {
                    failures.push(format!(
                        "plain empty list retained: {strict}/{mode}: {result}"
                    ));
                }
                let reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
                assert!(reopened.workbook().sheets[0].hyperlinks.is_empty());
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
