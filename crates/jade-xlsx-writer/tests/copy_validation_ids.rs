//! Copied input rules and sparkline groups have independent revision identities.
mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{
    data_validations::all_supported_validation_containers,
    worksheet_extensions::{self, Kind},
};
use quick_xml::{events::Event, NsReader, Reader};
use std::collections::{BTreeMap, BTreeSet};
const DV: &str = "{CCE6A557-97BC-4B89-ADB6-D9C93CAAB3DF}";
const SP: &str = "{05C60535-1F16-4FD2-B633-F4F36F0B64E0}";
const IDS: [&str; 4] = [
    "{4A414445-584C-4000-8000-000000000001}",
    "{4A414445-584C-4000-8000-000000000002}",
    "{4A414445-584C-4000-8000-000000000003}",
    "{4A414445-584C-4000-8000-000000000004}",
];
fn open(ns: &str, alias: &str, body: &str) -> HeadlessWorkbook {
    let sheet = format!(
        r#"<worksheet xmlns="{ns}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:{alias}="http://schemas.microsoft.com/office/spreadsheetml/2014/revision" xmlns:z="http://schemas.microsoft.com/office/spreadsheetml/2015/revision2" xmlns:u="urn:foreign" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="u" mc:ProcessContent="u:pass"><sheetData/>{body}</worksheet>"#
    );
    HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet)).unwrap()
}
fn child_ids(xml: &str, name: &[u8], key: &str) -> Vec<String> {
    let mut r = Reader::from_str(xml);
    let mut depth = 0;
    let mut ids = Vec::new();
    loop {
        match r.read_event().unwrap() {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                if depth == 1 && e.name().as_ref() == name {
                    for a in e.attributes() {
                        let a = a.unwrap();
                        if a.key.as_ref() == key.as_bytes() {
                            ids.push(a.unescape_value().unwrap().into_owned());
                        }
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => depth -= 1,
            Event::Eof => return ids,
            _ => {}
        }
    }
}
fn owned_ids(xml: &str, alias: &str) -> Vec<String> {
    let mut containers = BTreeMap::new();
    for record in all_supported_validation_containers(xml).unwrap() {
        containers.insert(record.container.start, (record.container, record.extended));
    }
    let mut ids = Vec::new();
    for (span, extended) in containers.into_values() {
        ids.extend(child_ids(
            &xml[span],
            if extended {
                b"x:dataValidation"
            } else {
                b"dataValidation"
            },
            &format!("{alias}:uid"),
        ));
    }
    let sources = worksheet_extensions::sources(xml).unwrap();
    let mut r = NsReader::from_str(xml);
    loop {
        let start = r.buffer_position() as usize;
        match r.read_event().unwrap() {
            Event::Start(e) if sources.owns_payload_at(start, Kind::Sparklines) => {
                r.read_to_end(e.name()).unwrap();
                ids.extend(child_ids(
                    &xml[start..r.buffer_position() as usize],
                    b"x:sparklineGroup",
                    "z:uid",
                ));
            }
            Event::Eof => return ids,
            _ => {}
        }
    }
}
#[test]
fn copied_input_rules_get_new_ids_across_compatible_container_shapes() {
    for ns in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        for alias in ["q", "rev"] {
            let foreign = format!("<u:data {alias}:uid=\"{}\"/>", IDS[0]);
            let opaque=format!("<ext uri=\"urn:opaque\"><x:dataValidations count=\"1\"><x:dataValidation {alias}:uid=\"{}\" type=\"custom\"><x:formula1><m:f>1</m:f></x:formula1><m:sqref>C2</m:sqref></x:dataValidation></x:dataValidations></ext>",IDS[3]);
            let body=format!("<dataValidations count=\"1\"><dataValidation {alias}:uid=\"{}\" type=\"custom\" sqref=\"B2\" u:uid=\"keep\"><formula1>1</formula1>{foreign}</dataValidation></dataValidations><extLst>{opaque}<ext uri=\"{DV}\"><x:dataValidations count=\"1\"><x:dataValidation {alias}:uid=\"{}\" type=\"custom\" u:uid=\"keep\"><x:formula1><m:f>1</m:f></x:formula1><m:sqref>C2</m:sqref>{foreign}</x:dataValidation></x:dataValidations></ext><ext uri=\"{SP}\"><x:sparklineGroups><x:sparklineGroup z:uid=\"{}\"><x:sparklines><x:sparkline><m:f>Sheet1!A1:A3</m:f><m:sqref>D2</m:sqref></x:sparkline></x:sparklines></x:sparklineGroup></x:sparklineGroups></ext></extLst>",IDS[0],IDS[1],IDS[2]);
            let ignored=format!("<mc:Choice Requires=\"u\"><dataValidations count=\"1\"><dataValidation {alias}:uid=\"{}\" type=\"custom\" sqref=\"Z2\"><formula1>1</formula1></dataValidation></dataValidations></mc:Choice>",IDS[3]);
            for mode in 0..3 {
                let wrapped=match mode {0=>body.clone(),1=>format!("<u:pass>{body}</u:pass>"),_=>format!("<mc:AlternateContent>{ignored}<mc:Fallback>{body}</mc:Fallback></mc:AlternateContent>")};
                let mut book = open(ns, alias, &wrapped);
                assert_eq!(book.workbook().sheets[0].data_validations.len(), 2);
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
                let mut all = BTreeSet::new();
                for index in 1..=3 {
                    let xml = package
                        .required_part_str(&format!("xl/worksheets/sheet{index}.xml"))
                        .unwrap();
                    let ids = owned_ids(&xml, alias);
                    assert_eq!(ids.len(), 3, "{xml}");
                    for id in ids {
                        if index == 1 {
                            assert!(IDS[..3].contains(&id.as_str()));
                        } else {
                            assert!(!IDS.contains(&id.as_str()));
                        }
                        assert!(all.insert(id), "{xml}");
                    }
                    assert!(xml.contains(&foreign), "{xml}");
                    assert!(xml.contains(&opaque), "{xml}");
                    assert!(xml.contains("u:uid=\"keep\""), "{xml}");
                    if mode == 2 {
                        assert!(xml.contains(&ignored), "{xml}");
                    }
                    assert_eq!(book.workbook().sheets[index - 1].data_validations.len(), 2);
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
}
#[test]
fn optional_foreign_and_unprocessed_input_rule_ids_are_preserved() {
    for (uri, attrs) in [
        (DV, String::new()),
        (DV, format!("u:uid=\"{}\"", IDS[0])),
        (DV, format!("xmlns:q=\"urn:foreign\" q:uid=\"{}\"", IDS[0])),
        ("urn:opaque", format!("q:uid=\"{}\"", IDS[0])),
        (SP, format!("q:uid=\"{}\"", IDS[0])),
    ] {
        let payload=format!("<x:dataValidations count=\"1\"><x:dataValidation {attrs} type=\"custom\"><x:formula1><m:f>1</m:f></x:formula1><m:sqref>B2</m:sqref></x:dataValidation></x:dataValidations>");
        let mut book = open(
            "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
            "q",
            &format!("<extLst><ext uri=\"{uri}\">{payload}</ext></extLst>"),
        );
        book.copy_sheet("Sheet1", "Copy").unwrap();
        let copied = OpcPackage::open(&book.save_bytes().unwrap())
            .unwrap()
            .required_part_str("xl/worksheets/sheet2.xml")
            .unwrap();
        assert!(copied.contains(&payload), "{copied}");
    }
}

#[test]
fn splitting_input_rules_assigns_independent_revision_ids() {
    use jade_xlsx_reader::{Axis, CellRange};
    for ns in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        for alias in ["q", "rev"] {
            let foreign = format!("<u:data {alias}:uid=\"{}\"><dataValidation {alias}:uid=\"{}\" sqref=\"Z1\"/><cfRule priority=\"1\"/></u:data>", IDS[2], IDS[0]);
            let opaque = format!("<ext uri=\"urn:opaque\"><x:dataValidations count=\"1\"><x:dataValidation {alias}:uid=\"{}\" type=\"custom\"><x:formula1><m:f>1</m:f></x:formula1><m:sqref>B2:D4</m:sqref></x:dataValidation></x:dataValidations></ext>", IDS[2]);
            let body = format!("<dataValidations count=\"1\"><dataValidation {alias}:uid=\"{}\" type=\"custom\" sqref=\"B2:D4\" u:uid=\"keep\"><formula1>A1</formula1>{foreign}</dataValidation></dataValidations><extLst>{opaque}<ext uri=\"{DV}\"><x:dataValidations count=\"1\"><x:dataValidation {alias}:uid=\"{}\" type=\"custom\" u:uid=\"keep\"><x:formula1><m:f>A1</m:f></x:formula1><m:sqref>B2:D4</m:sqref>{foreign}</x:dataValidation></x:dataValidations></ext></extLst>", IDS[0], IDS[1]);
            for mode in 0..3 {
                let body = match mode {
                    1 => format!("<u:pass>{body}</u:pass>"),
                    2 => format!("<mc:AlternateContent><mc:Choice Requires=\"u\"><u:ignored/></mc:Choice><mc:Fallback>{body}</mc:Fallback></mc:AlternateContent>"),
                    _ => body.clone(),
                };
                for axis in [Axis::Rows, Axis::Columns] {
                    for inserted in [false, true] {
                        let mut book = open(ns, alias, &body);
                        let before = book.bytes().to_vec();
                        book.shift_cells("Sheet1", CellRange::parse("C3").unwrap(), axis, inserted)
                            .unwrap();
                        let after = book.bytes().to_vec();
                        book.undo().unwrap();
                        assert_eq!(book.bytes(), before);
                        book.redo().unwrap();
                        assert_eq!(book.bytes(), after);
                        let bytes = book.save_bytes().unwrap();
                        let xml = OpcPackage::open(&bytes)
                            .unwrap()
                            .required_part_str("xl/worksheets/sheet1.xml")
                            .unwrap();
                        let ids = owned_ids(&xml, alias);
                        assert!(
                            !xml.contains("xmlns:_jade_ctx"),
                            "temporary context leaked: {xml}"
                        );
                        assert!(ids.len() > 2, "{xml}");
                        assert_eq!(
                            ids.iter().collect::<BTreeSet<_>>().len(),
                            ids.len(),
                            "split rules share IDs: {xml}"
                        );
                        for source in &IDS[..2] {
                            assert_eq!(
                                ids.iter().filter(|id| id.as_str() == *source).count(),
                                1,
                                "{xml}"
                            );
                        }
                        assert!(!ids.iter().any(|id| id == IDS[2]), "{xml}");
                        assert!(xml.contains(&opaque), "{xml}");
                        assert_eq!(xml.matches(&foreign).count(), ids.len(), "{xml}");
                        assert_eq!(xml.matches("u:uid=\"keep\"").count(), ids.len(), "{xml}");
                        let mut reopened = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
                        let saved = OpcPackage::open(&reopened.save_bytes().unwrap())
                            .unwrap()
                            .required_part_str("xl/worksheets/sheet1.xml")
                            .unwrap();
                        assert_eq!(saved, xml);
                    }
                }
            }
        }
    }
}
