use jade_opc::OpcPackage;
use jade_pptx_reader::Transform;
use jade_pptx_writer::{editable_shapes, shape_text, EditSession, TextFormat};

const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn bounds() -> Transform {
    Transform {
        offset_emu: (952500, 952500),
        extent_emu: (3048000, 1143000),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    }
}
fn xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap()
}
fn with_xml(value: &str) -> Vec<u8> {
    let mut package = OpcPackage::open(DECK).unwrap();
    package.replace("ppt/slides/slide1.xml", value.as_bytes().to_vec());
    package.write().unwrap()
}

#[test]
fn insertion_preserves_original_xml_and_other_parts_and_has_one_history_action() {
    let mut session = EditSession::open(DECK).unwrap();
    let old = xml(DECK);
    let text = "\n日本語 & <text> 🦀\n é👩‍💻 \n";
    let id = session
        .insert_text_box(
            0,
            bounds(),
            text,
            &TextFormat {
                bold: Some(true),
                size_pt: Some(24.0),
                color: Some("#12AB34".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let inserted = session.bytes().to_vec();
    let snapshot = shape_text(&inserted, 0, id).unwrap();
    assert!(snapshot.can_edit);
    assert_eq!(snapshot.text, text);
    assert_eq!(
        editable_shapes(&inserted, 0).unwrap().last().unwrap().id,
        id
    );
    assert_eq!(
        editable_shapes(&inserted, 0)
            .unwrap()
            .last()
            .unwrap()
            .transform,
        Some(bounds())
    );
    let changed = xml(&inserted);
    let start = changed.find("<jadeSp:sp ").unwrap();
    let end = changed[start..].find("</jadeSp:sp>").unwrap() + start + "</jadeSp:sp>".len();
    assert_eq!(format!("{}{}", &changed[..start], &changed[end..]), old);
    let before = OpcPackage::open(DECK).unwrap();
    let after = OpcPackage::open(&inserted).unwrap();
    assert_eq!(before.part_names().len(), after.part_names().len());
    for part in before
        .part_names()
        .iter()
        .filter(|part| part.as_str() != "ppt/slides/slide1.xml")
    {
        assert_eq!(before.part(part), after.part(part), "{part}");
    }
    assert!(changed.contains("val=\"12AB34\""));
    assert!(session.undo());
    assert_eq!(session.bytes(), DECK);
    assert!(!session.undo());
    assert!(session.redo());
    assert_eq!(session.bytes(), inserted);
    let reopened = EditSession::open(session.bytes()).unwrap();
    assert_eq!(shape_text(reopened.bytes(), 0, id).unwrap().text, text);
}

#[test]
fn empty_text_boxes_accept_typing_with_their_initial_style() {
    let mut session = EditSession::open(DECK).unwrap();
    let id = session
        .insert_text_box(
            0,
            bounds(),
            "",
            &TextFormat {
                italic: Some(true),
                font: Some("A & B \"Quoted\"".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let inserted = session.bytes().to_vec();
    assert_eq!(shape_text(&inserted, 0, id).unwrap().text, "");
    session.set_shape_text(0, id, "typed", false).unwrap();
    let run = jade_pptx_writer::editable_runs(session.bytes(), 0)
        .unwrap()
        .into_iter()
        .find(|run| run.address.shape_id == id)
        .unwrap();
    assert_eq!(run.text, "typed");
    assert!(xml(session.bytes()).contains("sz=\"1800\""));
    assert!(xml(session.bytes()).contains("i=\"1\""));
    assert!(xml(session.bytes()).contains("A &amp; B &quot;Quoted&quot;"));
    assert!(session.undo());
    assert_eq!(session.bytes(), inserted);
    assert!(session.undo());
    assert_eq!(session.bytes(), DECK);
}

#[test]
fn invalid_requests_do_not_mutate_bytes_or_history() {
    let mut session = EditSession::open(DECK).unwrap();
    for transform in [
        Transform {
            extent_emu: (0, 10),
            ..bounds()
        },
        Transform {
            extent_emu: (10, -1),
            ..bounds()
        },
        Transform {
            offset_emu: (i64::MAX, 0),
            ..bounds()
        },
    ] {
        assert!(session
            .insert_text_box(0, transform, "", &TextFormat::default())
            .is_err());
    }
    assert!(session
        .insert_text_box(0, bounds(), "bad\u{1}", &TextFormat::default())
        .is_err());
    assert!(session
        .insert_text_box(
            0,
            bounds(),
            "ok",
            &TextFormat {
                size_pt: Some(f64::NAN),
                ..Default::default()
            }
        )
        .is_err());
    assert!(session
        .insert_text_box(99, bounds(), "", &TextFormat::default())
        .is_err());
    assert_eq!(session.bytes(), DECK);
    assert!(!session.undo());
}

#[test]
fn insertion_supports_strict_and_aliased_namespaces_and_preserves_tree_extensions() {
    for strict in [false, true] {
        let mut value = xml(DECK)
            .replace("<p:", "<s:")
            .replace("</p:", "</s:")
            .replace("xmlns:p=", "xmlns:s=")
            .replace("<a:", "<d:")
            .replace("</a:", "</d:")
            .replace("xmlns:a=", "xmlns:d=");
        if strict {
            value = value
                .replace(
                    "http://schemas.openxmlformats.org/presentationml/2006/main",
                    "http://purl.oclc.org/ooxml/presentationml/main",
                )
                .replace(
                    "http://schemas.openxmlformats.org/drawingml/2006/main",
                    "http://purl.oclc.org/ooxml/drawingml/main",
                );
        }
        let extension =
            "<s:extLst><s:ext uri=\"test\"><payload xmlns=\"urn:test\"/></s:ext></s:extLst>";
        value = value.replace("</s:spTree>", &format!("{extension}</s:spTree>"));
        let bytes = with_xml(&value);
        let mut session = EditSession::open(&bytes).unwrap();
        let id = session
            .insert_text_box(0, bounds(), "aliased", &TextFormat::default())
            .unwrap();
        let changed = xml(session.bytes());
        assert!(changed.contains(&format!("</jadeSp:sp>{extension}")));
        assert_eq!(shape_text(session.bytes(), 0, id).unwrap().text, "aliased");
        assert_eq!(
            changed.contains("xmlns:jadeSp=\"http://purl.oclc.org"),
            strict
        );
    }
}

#[test]
fn allocated_ids_avoid_group_children_and_dangling_references_and_names_are_unique() {
    let extra = r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="50" name="TextBox 1"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="51" name="Child"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/></p:sp></p:grpSp>"#;
    let bytes = with_xml(&xml(DECK).replace("</p:spTree>", &format!("{extra}</p:spTree>")).replace("</p:sld>", "<p:extLst><p:ext uri=\"test\"><ref xmlns=\"urn:test\" shapeId=\"999\"/></p:ext></p:extLst></p:sld>"));
    let mut session = EditSession::open(&bytes).unwrap();
    let first = session
        .insert_text_box(0, bounds(), "one", &TextFormat::default())
        .unwrap();
    let second = session
        .insert_text_box(0, bounds(), "two", &TextFormat::default())
        .unwrap();
    assert_eq!((first, second), (1000, 1001));
    assert!(xml(session.bytes()).contains("name=\"TextBox 2\""));
    assert!(xml(session.bytes()).contains("name=\"TextBox 3\""));
}
