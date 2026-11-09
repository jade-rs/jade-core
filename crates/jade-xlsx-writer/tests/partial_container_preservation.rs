//! Deleting supported records must retain their unprocessed sibling data.
mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{Axis, CellRange};
#[test]
fn deleting_last_partial_rule_preserves_opaque_container_siblings() {
    for namespace in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        for axis in [Axis::Rows, Axis::Columns] {
            for (uri, payload) in [
                (
                    "{78C0D931-6437-407D-A8EE-F0AAD7539E65}",
                    r#"<x:conditionalFormattings><x:conditionalFormatting><x:cfRule type="expression" priority="1"><m:f>1</m:f></x:cfRule><m:sqref>B2</m:sqref></x:conditionalFormatting><u:data>B2</u:data><!--keep--></x:conditionalFormattings>"#,
                ),
                (
                    "{CCE6A557-97BC-4B89-ADB6-D9C93CAAB3DF}",
                    r#"<x:dataValidations count="1"><x:dataValidation type="custom"><x:formula1><m:f>1</m:f></x:formula1><m:sqref>B2</m:sqref></x:dataValidation><u:data>B2</u:data><!--keep--></x:dataValidations>"#,
                ),
                (
                    "{05C60535-1F16-4FD2-B633-F4F36F0B64E0}",
                    r#"<x:sparklineGroups><x:sparklineGroup><x:sparklines><x:sparkline><m:f>Sheet1!A1:A3</m:f><m:sqref>B2</m:sqref></x:sparkline><u:data>B2</u:data><!--keep--></x:sparklines></x:sparklineGroup></x:sparklineGroups>"#,
                ),
            ] {
                let sheet = format!(
                    r#"<worksheet xmlns="{namespace}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:u="urn:foreign"><sheetData/><extLst><ext uri="{uri}">{payload}</ext></extLst></worksheet>"#
                );
                let mut book =
                    HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet))
                        .unwrap();
                let before = book.bytes().to_vec();
                book.shift_cells("Sheet1", CellRange::parse("B2").unwrap(), axis, false)
                    .unwrap();
                let after = book.bytes().to_vec();
                if let Some(directory) = std::env::var_os("JADE_SCHEMA_AUDIT_DIR") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    let kind = if payload.contains("conditionalFormattings") {
                        "conditional"
                    } else if payload.contains("dataValidations") {
                        "validation"
                    } else {
                        "sparkline"
                    };
                    std::fs::write(
                        directory.join(format!(
                            "partial-extension-{}-{axis:?}-{kind}.xlsx",
                            namespace.contains("purl.oclc.org")
                        )),
                        book.save_bytes().unwrap(),
                    )
                    .unwrap();
                }
                book.undo().unwrap();
                assert_eq!(book.bytes(), before);
                book.redo().unwrap();
                assert_eq!(book.bytes(), after);
                let xml = OpcPackage::open(&book.save_bytes().unwrap())
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                assert!(
                    xml.contains("<u:data>B2</u:data><!--keep-->"),
                    "{uri}: {xml}"
                );
                assert!(!xml.contains("<m:sqref>B2</m:sqref>"), "{uri}: {xml}");
                let mut reopened =
                    HeadlessWorkbook::open_for_editing(&book.save_bytes().unwrap()).unwrap();
                let saved = OpcPackage::open(&reopened.save_bytes().unwrap())
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                assert_eq!(saved, xml);
            }
        }
    }
}

#[test]
fn deleting_last_merge_keeps_opaque_container_data() {
    for namespace in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        for axis in [Axis::Rows, Axis::Columns] {
            for (attrs, opaque) in [
                ("", ""),
                (" u:flag=\"keep\"", ""),
                ("", "<!--keep-->"),
                ("", "<u:payload>B2:C2</u:payload>"),
                ("", "<u:mergeCell ref=\"B2:C2\"/>"),
            ] {
                let sheet = format!(
                    r#"<worksheet xmlns="{namespace}" xmlns:u="urn:foreign"><sheetData/><mergeCells count="1"{attrs}><mergeCell ref="B2:C2"/>{opaque}</mergeCells></worksheet>"#
                );
                let mut book =
                    HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet))
                        .unwrap();
                let before = book.bytes().to_vec();
                book.shift_cells("Sheet1", CellRange::parse("B2:C2").unwrap(), axis, false)
                    .unwrap();
                let after = book.bytes().to_vec();
                book.undo().unwrap();
                assert_eq!(book.bytes(), before);
                book.redo().unwrap();
                assert_eq!(book.bytes(), after);
                let bytes = book.save_bytes().unwrap();
                if let Some(directory) = std::env::var_os("JADE_SCHEMA_AUDIT_DIR") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    let kind = if !attrs.is_empty() {
                        "attribute"
                    } else if opaque.is_empty() {
                        "plain"
                    } else if opaque.starts_with("<!--") {
                        "comment"
                    } else if opaque.contains("u:payload") {
                        "payload"
                    } else {
                        "lookalike"
                    };
                    std::fs::write(
                        directory.join(format!(
                            "partial-merge-{}-{axis:?}-{kind}.xlsx",
                            namespace.contains("purl.oclc.org")
                        )),
                        &bytes,
                    )
                    .unwrap();
                }
                let saved = OpcPackage::open(&bytes)
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                if attrs.is_empty() && opaque.is_empty() {
                    assert!(!saved.contains("mergeCells"), "{saved}");
                } else {
                    assert!(
                        saved.contains(&format!(
                            "<mergeCells count=\"0\"{attrs}>{opaque}</mergeCells>"
                        )),
                        "{saved}"
                    );
                }
                let mut reopened = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
                let saved_again = OpcPackage::open(&reopened.save_bytes().unwrap())
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                assert_eq!(saved_again, saved);
            }
        }
    }
}

use quick_xml::{events::Event, name::ResolveResult, NsReader};
fn metadata_capsule(xml: &str) -> String {
    let mut reader = NsReader::from_str(xml);
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event().unwrap() {
            Event::Start(e)
                if e.local_name().as_ref() == b"AlternateContent"
                    && matches!(reader.resolver().resolve_element(e.name()).0, ResolveResult::Bound(n) if n.as_ref() == jade_xlsx_reader::data_validations::MC_NAMESPACE) =>
            {
                reader.read_to_end(e.name()).unwrap();
                return xml[start..reader.buffer_position() as usize].into();
            }
            Event::Eof => panic!("missing metadata capsule: {xml}"),
            _ => {}
        }
    }
}
fn worksheet_xml(book: &mut HeadlessWorkbook) -> String {
    OpcPackage::open(&book.save_bytes().unwrap())
        .unwrap()
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap()
}

#[test]
fn inactive_metadata_stays_opaque_during_later_grid_edits() {
    for namespace in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        let sheet = format!(
            r#"<worksheet xmlns="{namespace}" xmlns:u="urn:foreign" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="u"><sheetData/><mergeCells count="1" u:flag="keep"><mergeCell ref="B2:C2"/><!--keep--><u:mergeCell ref="B2:C2"/><u:payload><mergeCells count="1"><mergeCell ref="F4:G4"/></mergeCells><hyperlinks><hyperlink ref="F4" location="Sheet1!B2"/></hyperlinks><autoFilter ref="B2:C2"/><f>Sheet1!F4</f></u:payload></mergeCells></worksheet>"#
        );
        let mut book =
            HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet)).unwrap();
        book.shift_cells(
            "Sheet1",
            CellRange::parse("B2:C2").unwrap(),
            Axis::Rows,
            false,
        )
        .unwrap();
        assert!(book.workbook().sheets[0].merges.is_empty());
        let original = metadata_capsule(&worksheet_xml(&mut book));
        book.insert_rows("Sheet1", 1, 1).unwrap();
        assert_eq!(metadata_capsule(&worksheet_xml(&mut book)), original);
        book.insert_columns("Sheet1", "A", 1).unwrap();
        assert_eq!(metadata_capsule(&worksheet_xml(&mut book)), original);
        book.rename_sheet("Sheet1", "Renamed").unwrap();
        assert_eq!(metadata_capsule(&worksheet_xml(&mut book)), original);
        book.shift_cells(
            "Renamed",
            CellRange::parse("A1").unwrap(),
            Axis::Columns,
            true,
        )
        .unwrap();
        assert_eq!(metadata_capsule(&worksheet_xml(&mut book)), original);
        let bytes = book.save_bytes().unwrap();
        let mut reopened = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
        assert!(reopened.workbook().sheets[0].merges.is_empty());
        assert_eq!(metadata_capsule(&worksheet_xml(&mut reopened)), original);
        if let Some(directory) = std::env::var_os("JADE_SCHEMA_AUDIT_DIR") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join(format!(
                    "inactive-later-edits-{}.xlsx",
                    namespace.contains("purl.oclc.org")
                )),
                &bytes,
            )
            .unwrap();
        }
    }
}

#[test]
fn inactive_group_metadata_stays_opaque_beside_a_live_extension_group() {
    for namespace in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        let sheet = format!(
            r#"<worksheet xmlns="{namespace}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:u="urn:foreign" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="u"><sheetData/><extLst><ext uri="{{05C60535-1F16-4FD2-B633-F4F36F0B64E0}}"><x:sparklineGroups><x:sparklineGroup u:flag="keep"><x:sparklines><x:sparkline><m:f>Sheet1!A1:A3</m:f><m:sqref>B2</m:sqref></x:sparkline><u:payload><mergeCells count="1"><mergeCell ref="F4:G4"/></mergeCells><autoFilter ref="F4:G4"/></u:payload></x:sparklines></x:sparklineGroup><x:sparklineGroup><x:sparklines><x:sparkline><m:f>Sheet1!A1:A3</m:f><m:sqref>D3</m:sqref></x:sparkline></x:sparklines></x:sparklineGroup></x:sparklineGroups></ext></extLst></worksheet>"#
        );
        let mut book =
            HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet)).unwrap();
        book.shift_cells("Sheet1", CellRange::parse("B2").unwrap(), Axis::Rows, false)
            .unwrap();
        assert_eq!(book.workbook().sheets[0].sparkline_groups.len(), 1);
        let original = metadata_capsule(&worksheet_xml(&mut book));
        book.insert_rows("Sheet1", 1, 1).unwrap();
        assert_eq!(metadata_capsule(&worksheet_xml(&mut book)), original);
        book.insert_columns("Sheet1", "A", 1).unwrap();
        assert_eq!(metadata_capsule(&worksheet_xml(&mut book)), original);
        book.rename_sheet("Sheet1", "Renamed").unwrap();
        let saved = worksheet_xml(&mut book);
        assert_eq!(metadata_capsule(&saved), original);
        assert!(saved.contains("<m:sqref>E4</m:sqref>"), "{saved}");
        assert!(saved.contains("<m:f>Renamed!"), "{saved}");
        assert_eq!(book.workbook().sheets[0].sparkline_groups.len(), 1);
        let bytes = book.save_bytes().unwrap();
        let mut reopened = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
        assert_eq!(metadata_capsule(&worksheet_xml(&mut reopened)), original);
        assert_eq!(reopened.workbook().sheets[0].sparkline_groups.len(), 1);
        if let Some(directory) = std::env::var_os("JADE_SCHEMA_AUDIT_DIR") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join(format!(
                    "inactive-live-group-{}.xlsx",
                    namespace.contains("purl.oclc.org")
                )),
                &bytes,
            )
            .unwrap();
        }
    }
}
