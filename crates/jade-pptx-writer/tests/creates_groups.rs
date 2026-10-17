use jade_opc::OpcPackage;
use jade_pptx_reader::ShapeItem;
use jade_pptx_writer::{editable_shapes, EditSession};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/grouped-deck.pptx");
fn xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap()
}
fn edited(value: &str) -> Vec<u8> {
    let mut p = OpcPackage::open(DECK).unwrap();
    p.replace("ppt/slides/slide1.xml", value.as_bytes().to_vec());
    p.write().unwrap()
}
#[test]
fn grouping_existing_groups_and_connectors_keeps_ids_geometry_references_and_all_other_parts() {
    let mut s = EditSession::open(DECK).unwrap();
    let id = s.group_objects(0, &[70, 50]).unwrap();
    let before = jade_pptx_reader::read_deck(DECK).unwrap();
    let after = jade_pptx_reader::read_deck(s.bytes()).unwrap();
    let ShapeItem::Group(group) = &after.slides[0].content.shapes[0] else {
        panic!("group");
    };
    assert_eq!(group.id, Some(id));
    assert_eq!(group.children, before.slides[0].content.shapes[..2]);
    let transform = group.transform.as_ref().unwrap();
    assert_eq!(transform.outer.offset_emu, transform.child_offset_emu);
    assert_eq!(transform.outer.extent_emu, transform.child_extent_emu);
    let old = jade_pptx_layout::layout_deck_slide(&before, 0, &jade_pptx_layout::HeuristicMeasure)
        .unwrap();
    let new = jade_pptx_layout::layout_deck_slide(&after, 0, &jade_pptx_layout::HeuristicMeasure)
        .unwrap();
    assert_eq!(old.shapes.len(), new.shapes.len());
    for (a, b) in old.shapes.iter().zip(&new.shapes) {
        assert_eq!(a.source_id, b.source_id);
        assert_eq!(a.lines, b.lines);
        let a = a.paint_transform();
        let b = b.paint_transform();
        assert!((a.center.0 - b.center.0).abs() < 1e-8);
        assert!((a.center.1 - b.center.1).abs() < 1e-8);
        assert_eq!(a.scale, b.scale);
        assert_eq!(a.rotation_rad, b.rotation_rad);
    }
    let a = OpcPackage::open(DECK).unwrap();
    let b = OpcPackage::open(s.bytes()).unwrap();
    for part in a
        .part_names()
        .iter()
        .filter(|p| p.as_str() != "ppt/slides/slide1.xml")
    {
        assert_eq!(a.part(part), b.part(part));
    }
    assert!(xml(s.bytes()).contains("<d:stCxn id=\"3\""));
    assert!(xml(s.bytes()).contains("spid=\"4\""));
    let grouped = s.bytes().to_vec();
    assert!(s.undo());
    assert_eq!(s.bytes(), DECK);
    assert!(!s.undo());
    assert!(s.redo());
    assert_eq!(s.bytes(), grouped);
}
#[test]
fn grouping_nonadjacent_objects_keeps_selected_order_and_uses_the_highest_selected_slot() {
    let mut s = EditSession::open(DECK).unwrap();
    let id = s.group_objects(0, &[99, 50]).unwrap();
    let deck = jade_pptx_reader::read_deck(s.bytes()).unwrap();
    let ShapeItem::Shape(outside) = &deck.slides[0].content.shapes[0] else {
        panic!("outside connector");
    };
    assert_eq!(outside.id, Some(70));
    let ShapeItem::Group(group) = &deck.slides[0].content.shapes[1] else {
        panic!("new group");
    };
    assert_eq!(group.id, Some(id));
    let ShapeItem::Group(first) = &group.children[0] else {
        panic!("old group first");
    };
    assert_eq!(first.id, Some(50));
    let ShapeItem::Shape(second) = &group.children[1] else {
        panic!("outside target second");
    };
    assert_eq!(second.id, Some(99));
}
#[test]
fn invalid_selections_placeholders_and_grouping_locks_do_not_change_bytes_or_history() {
    let mut s = EditSession::open(DECK).unwrap();
    for ids in [
        &[][..],
        &[50][..],
        &[50, 50][..],
        &[50, 51][..],
        &[50, 999][..],
    ] {
        assert!(s.group_objects(0, ids).is_err());
    }
    assert_eq!(s.bytes(), DECK);
    assert!(!s.undo());
    for lock in ["noGrp=\"1\"", "noSelect=\"true\""] {
        let bytes = edited(&xml(DECK).replacen(
            "<g:cNvGrpSpPr/>",
            &format!("<g:cNvGrpSpPr><d:grpSpLocks {lock}/></g:cNvGrpSpPr>"),
            1,
        ));
        let mut s = EditSession::open(&bytes).unwrap();
        assert!(s.group_objects(0, &[50, 70]).is_err());
        assert_eq!(s.bytes(), bytes);
        assert!(!s.undo());
    }
    let sample = include_bytes!("../../../public/fixtures/sample-deck.pptx");
    let shapes = editable_shapes(sample, 0).unwrap();
    assert!(!shapes.iter().find(|shape| shape.id == 2).unwrap().can_group);
    let mut s = EditSession::open(sample).unwrap();
    assert!(s.group_objects(0, &[2, 3]).is_err());
    assert_eq!(s.bytes(), sample);
}
#[test]
fn alternate_wrappers_move_whole_with_their_logical_ids_and_namespaces() {
    let original = xml(DECK);
    let start = original.find("<g:grpSp xmlns:g=").unwrap();
    let end = original.rfind("</g:grpSp>").unwrap() + "</g:grpSp>".len();
    let group = &original[start..end];
    let wrapper=format!("<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice xmlns:q=\"urn:test\" Requires=\"q\">{group}</mc:Choice><mc:Fallback>{group}</mc:Fallback></mc:AlternateContent>");
    let bytes = edited(&format!(
        "{}{}{}",
        &original[..start],
        wrapper,
        &original[end..]
    ));
    let mut s = EditSession::open(&bytes).unwrap();
    s.group_objects(0, &[50, 70]).unwrap();
    let value = xml(s.bytes());
    assert!(value.contains(group));
    assert!(value.contains("Requires=\"q\""));
    assert!(value.contains("xmlns:q=\"urn:test\""));
    assert_eq!(value.matches("id=\"50\"").count(), 2);
    assert_eq!(value.matches("<mc:AlternateContent").count(), 1);
}
#[test]
fn strict_group_namespaces_and_reserved_shape_ids_are_preserved() {
    let original=xml(DECK).replace("http://schemas.openxmlformats.org/presentationml/2006/main","http://purl.oclc.org/ooxml/presentationml/main").replace("http://schemas.openxmlformats.org/drawingml/2006/main","http://purl.oclc.org/ooxml/drawingml/main").replace("</p:sld>","<p:extLst><p:ext uri=\"test\"><q:ref xmlns:q=\"urn:test\" q:shapeId=\"100\"/></p:ext></p:extLst></p:sld>");
    let bytes = edited(&original);
    let mut s = EditSession::open(&bytes).unwrap();
    let id = s.group_objects(0, &[50, 70]).unwrap();
    assert_eq!(id, 101);
    assert!(xml(s.bytes())
        .contains("xmlns:jadeGroup=\"http://purl.oclc.org/ooxml/presentationml/main\""));
}
