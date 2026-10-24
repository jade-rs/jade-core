use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::{
    ChartChange, ChartKind, HeadlessError, HeadlessWorkbook, ShapeAnchor,
};
use jade_xlsx_reader::CellRef;
fn fixture() -> Vec<u8> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/objects-workbook.xlsx"
    ))
    .unwrap();
    let mut p = OpcPackage::open(&bytes).unwrap();
    let xml = p
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .replacen(
            "</sheetData>",
            "</sheetData><sheetProtection sheet=\"1\" objects=\"1\"/>",
            1,
        );
    p.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let xml = p
        .required_part_str("xl/drawings/drawing1.xml")
        .unwrap()
        .replace("<xdr:clientData", "<xdr:clientData fLocksWithSheet=\"0\"")
        .replacen("<xdr:sp ", "<xdr:sp fLocksText=\"0\" ", 1);
    p.replace("xl/drawings/drawing1.xml", xml.into_bytes());
    p.write().unwrap()
}
#[test]
fn individually_unlocked_objects_remain_editable_while_the_sheet_stays_protected() {
    let mut b = HeadlessWorkbook::open_for_editing(&fixture()).unwrap();
    assert!(
        !b.worksheet_protection("Data")
            .unwrap()
            .unwrap()
            .allowed
            .objects
    );
    let anchor = ShapeAnchor {
        from: CellRef::new(1, 2),
        from_offset: (0, 0),
        to: CellRef::new(5, 8),
        to_offset: (0, 0),
    };
    let p = OpcPackage::open(&b.export_bytes().unwrap()).unwrap();
    let source = jade_xlsx_reader::object_protection::drawing_source(
        &p.required_part_str("xl/drawings/drawing1.xml").unwrap(),
        2,
    )
    .unwrap()
    .unwrap();
    assert!(!source.locks.locked && !source.locks.text_locked);
    assert!(
        jade_xlsx_reader::object_protection::worksheet_drawing_relationship(
            &p.required_part_str("xl/worksheets/sheet1.xml").unwrap(),
            false
        )
        .unwrap()
        .is_some()
    );
    b.place_shape("Data", 2, &anchor).unwrap();
    b.set_shape_text("Data", "TextBox 1", "allowed").unwrap();
    b.set_chart(
        "Data",
        3,
        &ChartChange {
            title: Some(Some("allowed".into())),
            ..Default::default()
        },
    )
    .unwrap();
    b.delete_shape("Data", 4).unwrap();
    let bytes = b.export_bytes().unwrap();
    let mut reopened = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    assert!(
        !reopened
            .worksheet_protection("Data")
            .unwrap()
            .unwrap()
            .allowed
            .objects
    );
    reopened
        .set_shape_text("Data", "TextBox 1", "still allowed")
        .unwrap();
}
fn flags(locked: bool, text: bool) -> Vec<u8> {
    let mut p = OpcPackage::open(&fixture()).unwrap();
    let part = "xl/drawings/drawing1.xml";
    let xml = p
        .required_part_str(part)
        .unwrap()
        .replace(
            "fLocksWithSheet=\"0\"",
            &format!("fLocksWithSheet=\"{}\"", u8::from(locked)),
        )
        .replace(
            "fLocksText=\"0\"",
            &format!("fLocksText=\"{}\"", u8::from(text)),
        );
    p.replace(&part, xml.into_bytes());
    p.write().unwrap()
}
fn refused<T: std::fmt::Debug>(b: &HeadlessWorkbook, r: Result<T, HeadlessError>, before: &[u8]) {
    assert!(r.is_err(), "{r:?}");
    assert_eq!(b.export_bytes().unwrap(), before);
    assert!(!b.can_undo() && !b.can_redo());
}
fn moved() -> ShapeAnchor {
    ShapeAnchor {
        from: CellRef::new(1, 2),
        from_offset: (0, 0),
        to: CellRef::new(6, 8),
        to_offset: (0, 0),
    }
}
#[test]
fn drawing_position_and_text_flags_control_their_own_operations() {
    for locked in [false, true] {
        for text_locked in [false, true] {
            let bytes = flags(locked, text_locked);
            let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
            let before = b.export_bytes().unwrap();
            let r = b.place_shape("Data", 2, &moved());
            if locked {
                refused(&b, r, &before);
            } else {
                r.unwrap();
            }
            let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
            let before = b.export_bytes().unwrap();
            let r = b.set_shape_text("Data", "TextBox 1", "new text");
            if text_locked {
                refused(&b, r, &before);
            } else {
                r.unwrap();
                assert_eq!(
                    b.workbook().sheets[0]
                        .shape_named("TextBox 1")
                        .unwrap()
                        .text
                        .as_deref(),
                    Some("new text")
                );
            }
            let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
            let before = b.export_bytes().unwrap();
            let r = b.delete_shape("Data", 2);
            if locked {
                refused(&b, r, &before);
            } else {
                r.unwrap();
            }
            let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
            let before = b.export_bytes().unwrap();
            let r = b.insert_chart("Data", "A1:B3", ChartKind::Column);
            refused(&b, r, &before);
        }
    }
}
fn note_fixture(metadata: &str) -> Vec<u8> {
    let mut p = OpcPackage::open(&fixture()).unwrap();
    let sheet = "xl/worksheets/sheet1.xml";
    jade_xlsx_writer::set_note(&mut p, sheet, CellRef::new(3, 19), "original", "original").unwrap();
    let part = note_vml_part(&p);
    let xml = p.required_part_str(&part).unwrap();
    let note = jade_xlsx_reader::object_protection::note_sources(&xml)
        .unwrap()
        .into_iter()
        .find(|n| n.at == CellRef::new(3, 19))
        .unwrap();
    let replacement =
        xml[note.span.clone()].replace("</x:ClientData>", &format!("{metadata}</x:ClientData>"));
    let mut xml = xml;
    xml.replace_range(note.span, &replacement);
    p.replace(&part, xml.into_bytes());
    p.write().unwrap()
}
fn note_vml_part(p: &OpcPackage) -> String {
    let rels = jade_opc::parse_relationships(
        &p.required_part_str("xl/worksheets/_rels/sheet1.xml.rels")
            .unwrap(),
    )
    .unwrap();
    jade_opc::resolve_target(
        "xl/worksheets/sheet1.xml",
        rels.target_by_type_suffix("/vmlDrawing").unwrap(),
    )
}
#[test]
fn note_position_and_text_locks_are_independent_and_updates_keep_vml_bytes() {
    for locked in [false, true] {
        for text in [false, true] {
            let bytes = note_fixture(&format!(
                "<x:Locked>{}</x:Locked><x:LockText>{}</x:LockText>",
                if locked { "True" } else { "False" },
                if text { "True" } else { "False" }
            ));
            let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
            let before = b.export_bytes().unwrap();
            let p = OpcPackage::open(&before).unwrap();
            let part = note_vml_part(&p);
            let vml = p.required_part_str(&part).unwrap();
            let r = b.set_note("Data", "D20", "updated", "new author");
            if text {
                refused(&b, r, &before);
            } else {
                r.unwrap();
                let out = OpcPackage::open(&b.export_bytes().unwrap()).unwrap();
                assert_eq!(out.required_part_str(&part).unwrap(), vml);
                assert_eq!(
                    b.workbook().sheets[0]
                        .comment_at(CellRef::new(3, 19))
                        .unwrap()
                        .text,
                    "updated"
                );
            }
            let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
            let before = b.export_bytes().unwrap();
            let r = b.delete_note("Data", "D20");
            if locked {
                refused(&b, r, &before);
            } else {
                assert!(r.unwrap());
            }
        }
    }
}
#[test]
fn guards_follow_drawing_history_and_authentic_sheet_state_instead_of_stale_flags() {
    let bytes = flags(false, false);
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let before = b.export_bytes().unwrap();
    b.place_shape("Data", 2, &moved()).unwrap();
    let after = b.export_bytes().unwrap();
    assert!(b.undo().unwrap());
    assert_eq!(b.export_bytes().unwrap(), before);
    assert!(b.redo().unwrap());
    assert_eq!(b.export_bytes().unwrap(), after);
    let mut b = HeadlessWorkbook::open_for_editing(&flags(true, true)).unwrap();
    assert!(b.unprotect_sheet("Data", "").unwrap());
    b.place_shape("Data", 2, &moved()).unwrap();
    assert!(b.undo().unwrap());
    assert!(b.undo().unwrap());
    let before = b.export_bytes().unwrap();
    assert!(matches!(
        b.place_shape("Data", 2, &moved()),
        Err(HeadlessError::Protected { .. })
    ));
    assert_eq!(b.export_bytes().unwrap(), before);
    assert!(b.can_redo());
}
#[test]
fn malformed_or_ambiguous_individual_metadata_never_grants_an_edit() {
    for metadata in [
        "<x:Locked>0</x:Locked><x:LockText>False</x:LockText>",
        "<x:Locked>False</x:Locked><x:LockText><x:Row>False</x:Row></x:LockText>",
        "<x:Locked>False</x:Locked><x:Locked>True</x:Locked><x:LockText>False</x:LockText>",
    ] {
        let mut b = HeadlessWorkbook::open_for_editing(&note_fixture(metadata)).unwrap();
        let before = b.export_bytes().unwrap();
        let r = b.set_note("Data", "D20", "forbidden", "author");
        refused(&b, r, &before);
        let r = b.delete_note("Data", "D20");
        refused(&b, r, &before);
    }
    let mut p = OpcPackage::open(&fixture()).unwrap();
    let part = "xl/drawings/drawing1.xml";
    let xml = p
        .required_part_str(part)
        .unwrap()
        .replace("fLocksWithSheet=\"0\"", "fLocksWithSheet=\"bad\"");
    p.replace(&part, xml.into_bytes());
    let mut b = HeadlessWorkbook::open_for_editing(&p.write().unwrap()).unwrap();
    let before = b.export_bytes().unwrap();
    let r = b.set_shape_text("Data", "TextBox 1", "forbidden");
    refused(&b, r, &before);
}
#[test]
fn note_locks_follow_row_movement_and_survive_reprotection_and_reopen() {
    let mut b = HeadlessWorkbook::open_for_editing(&note_fixture(
        "<x:Locked>False</x:Locked><x:LockText>False</x:LockText>",
    ))
    .unwrap();
    b.unprotect_sheet("Data", "").unwrap();
    b.insert_rows("Data", 5, 1).unwrap();
    b.protect_sheet(
        "Data",
        jade_xlsx_reader::SheetProtection::default(),
        "",
        &[0; 16],
    )
    .unwrap();
    b.set_note("Data", "D21", "moved", "author").unwrap();
    let mut b = HeadlessWorkbook::open_for_editing(&b.export_bytes().unwrap()).unwrap();
    assert!(b.delete_note("Data", "D21").unwrap());
}
#[test]
fn literal_xstrings_and_opaque_comments_survive_text_updates_and_last_note_deletion() {
    let bytes = note_fixture("<x:Locked>False</x:Locked><x:LockText>False</x:LockText>");
    let mut p = OpcPackage::open(&bytes).unwrap();
    let rels = jade_opc::parse_relationships(
        &p.required_part_str("xl/worksheets/_rels/sheet1.xml.rels")
            .unwrap(),
    )
    .unwrap();
    let part = jade_opc::resolve_target(
        "xl/worksheets/sheet1.xml",
        rels.target_by_type_suffix("/comments").unwrap(),
    );
    let mut xml = p.required_part_str(&part).unwrap();
    let mut others: Vec<_> = jade_xlsx_reader::object_protection::comments_source(&xml)
        .unwrap()
        .comments
        .into_iter()
        .filter(|c| c.at != Some(CellRef::new(3, 19)))
        .map(|c| c.element.span)
        .collect();
    others.sort_by_key(|r| r.start);
    for span in others.into_iter().rev() {
        xml.replace_range(span, "");
    }
    let opaque = r#"<f:comment xmlns:f="urn:foreign" ref="D20"><f:text>keep</f:text></f:comment>"#;
    xml = xml
        .replace("</commentList>", &format!("{opaque}</commentList>"))
        .replace(
            "</comments>",
            r#"<f:metadata xmlns:f="urn:foreign"/></comments>"#,
        );
    p.replace(&part, xml.into_bytes());
    let mut b = HeadlessWorkbook::open_for_editing(&p.write().unwrap()).unwrap();
    let value = " _x0041_ \u{7} ";
    let author = "_x0042_ author";
    b.set_note("Data", "D20", value, author).unwrap();
    let note = b.workbook().sheets[0]
        .comment_at(CellRef::new(3, 19))
        .unwrap();
    assert_eq!(note.text, value);
    assert_eq!(note.author.as_deref(), Some(author));
    assert!(b.delete_note("Data", "D20").unwrap());
    let out = OpcPackage::open(&b.export_bytes().unwrap()).unwrap();
    let xml = out.required_part_str(&part).unwrap();
    assert!(xml.contains(opaque));
    assert!(xml.contains("<f:metadata"));
    assert!(b.workbook().sheets[0].comments.is_empty());
}
#[test]
fn worksheet_bindings_win_over_unused_and_foreign_relationships_for_reading_and_writing() {
    let bytes = note_fixture("<x:Locked>False</x:Locked><x:LockText>False</x:LockText>");
    let mut p = OpcPackage::open(&bytes).unwrap();
    let sheet = "xl/worksheets/sheet1.xml";
    let rels_part = "xl/worksheets/_rels/sheet1.xml.rels";
    let id = jade_xlsx_reader::object_protection::worksheet_drawing_relationship(
        &p.required_part_str(sheet).unwrap(),
        false,
    )
    .unwrap()
    .unwrap();
    let unused_drawing = p
        .required_part_str("xl/drawings/drawing1.xml")
        .unwrap()
        .replace("TextBox 1", "unused text box");
    p.replace("xl/drawings/unused.xml", unused_drawing.as_bytes().to_vec());
    let vml_part = note_vml_part(&p);
    let unused_vml = p.required_part_str(&vml_part).unwrap();
    p.replace("xl/drawings/unused.vml", unused_vml.as_bytes().to_vec());
    let extra = format!(
        r#"<Relationship Id="000-unused-drawing" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/unused.xml"/><Relationship Id="000-unused-vml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing" Target="../drawings/unused.vml"/><f:Relationship xmlns:f="urn:foreign" Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/unused.xml"/>"#
    );
    let rels = p
        .required_part_str(rels_part)
        .unwrap()
        .replace("</Relationships>", &format!("{extra}</Relationships>"));
    p.replace(rels_part, rels.into_bytes());
    let ct=p.required_part_str("[Content_Types].xml").unwrap().replace("</Types>",r#"<Override PartName="/xl/drawings/unused.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/></Types>"#);
    p.replace("[Content_Types].xml", ct.into_bytes());
    let mut b = HeadlessWorkbook::open_for_editing(&p.write().unwrap()).unwrap();
    assert!(b.workbook().sheets[0].shape_named("TextBox 1").is_some());
    assert!(b.workbook().sheets[0]
        .shape_named("unused text box")
        .is_none());
    b.place_shape("Data", 2, &moved()).unwrap();
    b.set_note("Data", "D20", "new", "author").unwrap();
    b.delete_note("Data", "D20").unwrap();
    let out = OpcPackage::open(&b.export_bytes().unwrap()).unwrap();
    assert_eq!(
        out.required_part_str("xl/drawings/unused.xml").unwrap(),
        unused_drawing
    );
    assert_eq!(
        out.required_part_str("xl/drawings/unused.vml").unwrap(),
        unused_vml
    );
    assert!(out.required_part_str(rels_part).unwrap().contains(&extra));
}
