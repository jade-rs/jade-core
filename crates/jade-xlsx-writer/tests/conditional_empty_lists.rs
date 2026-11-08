//! Removing the last conditional block must not leave an invalid empty x14 list.
mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const URI: &str = "{78C0D931-6437-407D-A8EE-F0AAD7539E65}";
const RULE: &str = r#"<x:conditionalFormatting><x:cfRule type="expression" priority="1"><m:f>1</m:f></x:cfRule><m:sqref>B2</m:sqref></x:conditionalFormatting>"#;
fn sheet(ns: &str, tail: &str) -> Vec<u8> {
    common::workbook_with_sheet(&format!(
        r#"<worksheet xmlns="{ns}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:u="urn:foreign" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="u" mc:ProcessContent="u:pass"><sheetData/>{tail}</worksheet>"#
    ))
}
fn xml(book: &mut HeadlessWorkbook) -> String {
    OpcPackage::open(&book.save_bytes().unwrap())
        .unwrap()
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap()
}
#[test]
fn deleting_last_conditional_coverage_removes_empty_owned_lists() {
    for ns in [MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main"] {
        for columns in [false, true] {
            let tail = format!("<extLst><ext uri=\"{URI}\"><x:conditionalFormattings>{RULE}</x:conditionalFormattings></ext></extLst>");
            let mut book = HeadlessWorkbook::open_for_editing(&sheet(ns, &tail)).unwrap();
            let before = book.bytes().to_vec();
            if columns {
                book.delete_columns("Sheet1", "B", 1).unwrap();
            } else {
                book.delete_rows("Sheet1", 2, 1).unwrap();
            }
            let after = book.bytes().to_vec();
            book.undo().unwrap();
            assert_eq!(book.bytes(), before);
            book.redo().unwrap();
            assert_eq!(book.bytes(), after);
            let saved = xml(&mut book);
            assert!(!saved.contains("conditionalFormattings"), "{saved}");
            assert!(!saved.contains("<extLst"), "{saved}");
            let mut reopened =
                HeadlessWorkbook::open_for_editing(&book.save_bytes().unwrap()).unwrap();
            assert_eq!(xml(&mut reopened), saved);
        }
    }
}
#[test]
fn cleanup_preserves_foreign_data_comments_and_preexisting_empty_lists() {
    let unknown = format!("<ext uri=\"urn:unknown\"><x:conditionalFormattings>{RULE}</x:conditionalFormattings></ext>");
    let foreign = "<u:conditionalFormattings><m:sqref>B2</m:sqref></u:conditionalFormattings>";
    let tail = format!("<extLst>{unknown}<ext uri=\"{URI}\"><x:conditionalFormattings/>{foreign}<x:conditionalFormattings><!--keep-->{RULE}</x:conditionalFormattings><x:conditionalFormattings>{RULE}<u:data>B2</u:data></x:conditionalFormattings><x:conditionalFormattings>{RULE}</x:conditionalFormattings></ext></extLst>");
    let mut book = HeadlessWorkbook::open_for_editing(&sheet(MAIN, &tail)).unwrap();
    book.delete_rows("Sheet1", 2, 1).unwrap();
    let saved = xml(&mut book);
    assert!(saved.contains(&unknown), "{saved}");
    assert!(saved.contains(foreign), "{saved}");
    assert!(saved.contains("<x:conditionalFormattings/>"), "{saved}");
    assert!(
        saved.contains("<x:conditionalFormattings><!--keep--></x:conditionalFormattings>"),
        "{saved}"
    );
    assert!(
        saved.contains("<x:conditionalFormattings><u:data>B2</u:data></x:conditionalFormattings>"),
        "{saved}"
    );
    assert!(
        !saved.contains("<x:conditionalFormattings></x:conditionalFormattings>"),
        "{saved}"
    );
}
#[test]
fn cleanup_follows_process_content_ownership_without_removing_wrappers() {
    let tail = format!("<extLst><ext uri=\"{URI}\"><u:pass><x:conditionalFormattings>{RULE}</x:conditionalFormattings></u:pass><u:data>B2</u:data></ext></extLst>");
    let mut book = HeadlessWorkbook::open_for_editing(&sheet(MAIN, &tail)).unwrap();
    book.delete_rows("Sheet1", 2, 1).unwrap();
    let saved = xml(&mut book);
    assert!(!saved.contains("conditionalFormattings"), "{saved}");
    assert!(
        saved.contains("<u:pass></u:pass><u:data>B2</u:data>"),
        "{saved}"
    );
}

#[test]
fn deleting_last_conditional_coverage_keeps_metadata_inactive_and_schema_safe() {
    check_empty_extension(false);
}
#[test]
fn deleting_last_sparkline_coverage_keeps_metadata_inactive_and_schema_safe() {
    check_empty_extension(true);
}
fn check_empty_extension(sparkline: bool) {
    use jade_xlsx_reader::worksheet_extensions::{self, Kind};
    use quick_xml::{events::Event, NsReader};
    let (element, uri, rule, kind, label) = if sparkline {
        ("sparklineGroups", "{05C60535-1F16-4FD2-B633-F4F36F0B64E0}", "<x:sparklineGroup u:option=\"keep\"><x:sparklines u:inside=\"keep\"><x:sparkline><m:f>Sheet1!A1:A3</m:f><m:sqref>B2</m:sqref></x:sparkline><!--inner--><u:inner>B2</u:inner></x:sparklines></x:sparklineGroup>", Kind::Sparklines, "sparkline")
    } else {
        (
            "conditionalFormattings",
            URI,
            RULE,
            Kind::ConditionalFormats,
            "conditional",
        )
    };
    for ns in [MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main"] {
        for columns in [false, true] {
            for wrapper in 0..3 {
                let list = format!("<x:{element} u:flag=\"keep\">{rule}<!--keep--><u:data>B2</u:data></x:{element}>");
                let list = match wrapper {
                    1 => format!("<u:pass>{list}</u:pass>"),
                    2 => format!("<mc:AlternateContent><mc:Choice Requires=\"u\"><u:ignored/></mc:Choice><mc:Fallback>{list}</mc:Fallback></mc:AlternateContent>"),
                    _ => list,
                };
                let mut book = HeadlessWorkbook::open_for_editing(&sheet(
                    ns,
                    &format!("<extLst><ext uri=\"{uri}\">{list}</ext></extLst>"),
                ))
                .unwrap();
                let before = book.bytes().to_vec();
                if columns {
                    book.delete_columns("Sheet1", "B", 1).unwrap();
                } else {
                    book.delete_rows("Sheet1", 2, 1).unwrap();
                }
                let after = book.bytes().to_vec();
                book.undo().unwrap();
                assert_eq!(book.bytes(), before);
                book.redo().unwrap();
                assert_eq!(book.bytes(), after);
                let bytes = book.save_bytes().unwrap();
                if let Some(directory) = std::env::var_os("JADE_SCHEMA_AUDIT_DIR") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    std::fs::write(
                        directory.join(format!(
                            "whole-{label}-{}-{columns}-{wrapper}.xlsx",
                            ns != MAIN
                        )),
                        &bytes,
                    )
                    .unwrap();
                }
                let saved = xml(&mut book);
                assert!(saved.contains("u:flag=\"keep\""), "{saved}");
                assert!(saved.contains("<!--keep--><u:data>B2</u:data>"), "{saved}");
                if sparkline {
                    assert!(saved.contains("u:option=\"keep\""), "{saved}");
                    assert!(saved.contains("u:inside=\"keep\""), "{saved}");
                    assert!(
                        saved.contains("<!--inner--><u:inner>B2</u:inner>"),
                        "{saved}"
                    );
                }
                let sources = worksheet_extensions::sources(&saved).unwrap();
                let mut reader = NsReader::from_str(&saved);
                loop {
                    let start = reader.buffer_position() as usize;
                    match reader.read_event().unwrap() {
                        Event::Start(e) | Event::Empty(e)
                            if e.local_name().as_ref() == element.as_bytes() =>
                        {
                            assert!(!sources.owns_payload_at(start, kind), "{saved}");
                        }
                        Event::Eof => break,
                        _ => {}
                    }
                }
                let mut reopened = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
                assert_eq!(xml(&mut reopened), saved);
            }
        }
    }
}
