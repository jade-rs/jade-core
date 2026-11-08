//! Copied sparkline groups have workbook-distinct revision identifiers.
mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use quick_xml::{events::Event, Reader};
const ID: &str = "{4A414445-584C-4000-8000-000000000001}";
const URI: &str = "{05C60535-1F16-4FD2-B633-F4F36F0B64E0}";
fn open(ns: &str, alias: &str, uri: &str, payload: &str, opaque: &str) -> HeadlessWorkbook {
    let sheet = format!(
        r#"<worksheet xmlns="{ns}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:{alias}="http://schemas.microsoft.com/office/spreadsheetml/2015/revision2" xmlns:u="urn:foreign"><sheetData/><extLst>{opaque}<ext uri="{uri}">{payload}</ext></extLst></worksheet>"#
    );
    HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet)).unwrap()
}
fn group_id(xml: &str, alias: &str) -> String {
    let marker = format!("uri=\"{URI}\"");
    let at = xml
        .to_ascii_uppercase()
        .find(&marker.to_ascii_uppercase())
        .unwrap();
    let mut reader = Reader::from_str(&xml[at + marker.len()..]);
    loop {
        match reader.read_event().unwrap() {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"sparklineGroup" => {
                for attr in e.attributes() {
                    let attr = attr.unwrap();
                    if attr.key.as_ref() == format!("{alias}:uid").as_bytes() {
                        return attr.unescape_value().unwrap().into_owned();
                    }
                }
                panic!("group identity missing: {xml}");
            }
            Event::Eof => panic!("sparkline group missing: {xml}"),
            _ => {}
        }
    }
}
#[test]
fn copied_sparkline_group_gets_a_fresh_revision_identity() {
    for ns in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        for alias in ["q", "rev2"] {
            let foreign = format!("<u:data {alias}:uid=\"{ID}\"/>");
            let opaque=format!("<ext uri=\"urn:opaque\"><x:sparklineGroups><x:sparklineGroup {alias}:uid=\"{ID}\"/></x:sparklineGroups></ext>");
            let payload=format!("<x:sparklineGroups><x:sparklineGroup {alias}:uid=\"{ID}\" u:uid=\"keep\"><x:sparklines><x:sparkline><m:f>Sheet1!A1:A3</m:f><m:sqref>B2</m:sqref></x:sparkline></x:sparklines>{foreign}</x:sparklineGroup></x:sparklineGroups>");
            let mut book = open(ns, alias, URI, &payload, &opaque);
            for name in ["Copy", "Copy2"] {
                let before = book.bytes().to_vec();
                book.copy_sheet("Sheet1", name).unwrap();
                let after = book.bytes().to_vec();
                book.undo().unwrap();
                assert_eq!(book.bytes(), before);
                book.redo().unwrap();
                assert_eq!(book.bytes(), after);
            }
            let saved = book.save_bytes().unwrap();
            let package = OpcPackage::open(&saved).unwrap();
            let mut ids = std::collections::BTreeSet::new();
            for index in 1..=3 {
                let xml = package
                    .required_part_str(&format!("xl/worksheets/sheet{index}.xml"))
                    .unwrap();
                let id = group_id(&xml, alias);
                if index == 1 {
                    assert_eq!(id, ID);
                } else {
                    assert_ne!(id, ID);
                }
                assert!(ids.insert(id));
                assert!(xml.contains(&opaque), "{xml}");
                assert!(xml.contains(&foreign), "{xml}");
                assert!(xml.contains("u:uid=\"keep\""), "{xml}");
                assert!(xml.contains("<m:sqref>B2</m:sqref>"), "{xml}");
                assert!(xml.contains("<m:f>Sheet1!A1:A3</m:f>"), "{xml}");
            }
            let mut reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
            let again = OpcPackage::open(&reopened.save_bytes().unwrap()).unwrap();
            for index in 1..=3 {
                let part = format!("xl/worksheets/sheet{index}.xml");
                assert_eq!(
                    again.required_part_str(&part).unwrap(),
                    package.required_part_str(&part).unwrap()
                );
            }
        }
    }
}
#[test]
fn optional_and_unprocessed_group_identity_attributes_are_unchanged() {
    for (uri, attrs) in [
        (URI, String::new()),
        (URI, format!("u:uid=\"{ID}\"")),
        (URI, format!("xmlns:q=\"urn:foreign\" q:uid=\"{ID}\"")),
        ("urn:opaque", format!("q:uid=\"{ID}\"")),
        (
            "{78C0D931-6437-407D-A8EE-F0AAD7539E65}",
            format!("q:uid=\"{ID}\""),
        ),
    ] {
        let payload=format!("<x:sparklineGroups><x:sparklineGroup {attrs}><x:sparklines><x:sparkline><m:f>Sheet1!A1:A3</m:f><m:sqref>B2</m:sqref></x:sparkline></x:sparklines></x:sparklineGroup></x:sparklineGroups>");
        let mut book = open(
            "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
            "q",
            uri,
            &payload,
            "",
        );
        book.copy_sheet("Sheet1", "Copy").unwrap();
        let package = OpcPackage::open(&book.save_bytes().unwrap()).unwrap();
        let copied = package
            .required_part_str("xl/worksheets/sheet2.xml")
            .unwrap();
        assert!(copied.contains(&payload), "{copied}");
    }
}
