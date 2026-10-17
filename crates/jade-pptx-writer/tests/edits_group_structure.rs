use jade_opc::OpcPackage;
use jade_pptx_reader::{GroupShape, ShapeItem};
use jade_pptx_writer::{editable_shapes, EditSession};
use std::collections::HashSet;
const DECK: &[u8] = include_bytes!("../../../public/fixtures/grouped-deck.pptx");
fn slide_xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap()
}
fn edited(xml: &str) -> Vec<u8> {
    let mut p = OpcPackage::open(DECK).unwrap();
    p.replace("ppt/slides/slide1.xml", xml.as_bytes().to_vec());
    p.write().unwrap()
}
fn groups(bytes: &[u8]) -> Vec<GroupShape> {
    jade_pptx_reader::read_deck(bytes).unwrap().slides[0]
        .content
        .shapes
        .iter()
        .filter_map(|item| match item {
            ShapeItem::Group(g) => Some(g.clone()),
            _ => None,
        })
        .collect()
}
fn ids(group: &GroupShape) -> HashSet<u32> {
    fn collect(items: &[ShapeItem], ids: &mut HashSet<u32>) {
        for item in items {
            match item {
                ShapeItem::Group(g) => {
                    ids.insert(g.id.unwrap());
                    collect(&g.children, ids);
                }
                ShapeItem::Shape(s) => {
                    ids.insert(s.id.unwrap());
                }
                ShapeItem::Picture(p) => {
                    ids.insert(p.id.unwrap());
                }
                _ => panic!("fixture"),
            }
        }
    }
    let mut ids = HashSet::from([group.id.unwrap()]);
    collect(&group.children, &mut ids);
    ids
}
fn source_group(xml: &str) -> &str {
    let start = xml.find("<g:grpSp xmlns:g=").unwrap();
    let end = xml.rfind("</g:grpSp>").unwrap() + "</g:grpSp>".len();
    &xml[start..end]
}
fn other_parts(before: &[u8], after: &[u8]) {
    let a = OpcPackage::open(before).unwrap();
    let b = OpcPackage::open(after).unwrap();
    for part in a
        .part_names()
        .iter()
        .filter(|p| p.as_str() != "ppt/slides/slide1.xml")
    {
        assert_eq!(a.part(part), b.part(part), "{part}");
    }
}

#[test]
fn nested_group_copy_remaps_every_id_but_only_offsets_the_outer_frame_and_keeps_source_parts() {
    let original = groups(DECK).remove(0);
    let mut s = EditSession::open(DECK).unwrap();
    let copy = s.duplicate_shape(0, 50).unwrap();
    let groups = groups(s.bytes());
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0], original);
    let clone = &groups[1];
    assert_eq!(clone.id, Some(copy));
    assert!(ids(&original).is_disjoint(&ids(clone)));
    assert_eq!(ids(&original).len(), ids(clone).len());
    let old = original.transform.as_ref().unwrap();
    let new = clone.transform.as_ref().unwrap();
    assert_eq!(
        new.outer.offset_emu,
        (
            old.outer.offset_emu.0 + 127000,
            old.outer.offset_emu.1 + 127000
        )
    );
    assert_eq!(new.child_offset_emu, old.child_offset_emu);
    assert_eq!(new.child_extent_emu, old.child_extent_emu);
    let (ShapeItem::Group(a), ShapeItem::Group(b)) = (&original.children[0], &clone.children[0])
    else {
        panic!("inner group");
    };
    assert_eq!(a.transform, b.transform);
    let (ShapeItem::Picture(a), ShapeItem::Picture(b)) = (&a.children[0], &b.children[0]) else {
        panic!("picture");
    };
    assert_eq!(a.transform, b.transform);
    assert_eq!(a.image_part, b.image_part);
    assert_eq!(a.blip_rel_id, b.blip_rel_id);
    assert!(slide_xml(s.bytes()).contains(source_group(&slide_xml(DECK))));
    assert_eq!(
        slide_xml(s.bytes())
            .matches("{0980FF19-E7E7-493C-8D3E-15B2100EA940}")
            .count(),
        1
    );
    other_parts(DECK, s.bytes());
    let bytes = s.bytes().to_vec();
    assert!(s.undo());
    assert_eq!(s.bytes(), DECK);
    assert!(!s.undo());
    assert!(s.redo());
    assert_eq!(s.bytes(), bytes);
}
#[test]
fn internal_connectors_and_text_builds_follow_the_copy_while_external_connectors_keep_the_original()
{
    let mut s = EditSession::open(DECK).unwrap();
    let copy = s.duplicate_shape(0, 50).unwrap();
    let groups = groups(s.bytes());
    let clone = &groups[1];
    let ShapeItem::Group(inner) = &clone.children[0] else {
        panic!("inner");
    };
    let ShapeItem::Picture(red) = &inner.children[0] else {
        panic!("red");
    };
    let ShapeItem::Shape(text) = &clone.children[2] else {
        panic!("text");
    };
    let xml = slide_xml(s.bytes());
    assert!(xml.contains(&format!("<d:stCxn id=\"{}\"", red.id.unwrap())));
    assert_eq!(xml.matches("<d:endCxn id=\"99\"").count(), 3);
    assert_eq!(xml.matches("<d:stCxn id=\"3\"").count(), 1);
    assert!(xml.contains(&format!("spid=\"{}\"", text.id.unwrap())));
    assert!(xml.contains("spid=\"4\""));
    assert!(editable_shapes(s.bytes(), 0)
        .unwrap()
        .iter()
        .any(|g| g.id == copy));
}
#[test]
fn deletion_detaches_child_connections_removes_owned_builds_and_preserves_the_copy_and_media() {
    let mut s = EditSession::open(DECK).unwrap();
    let copy = s.duplicate_shape(0, 50).unwrap();
    let before = s.bytes().to_vec();
    let clone = groups(&before)[1].clone();
    assert!(s.delete_shape(0, 50).unwrap());
    assert_eq!(groups(s.bytes()), vec![clone]);
    let value = slide_xml(s.bytes());
    assert!(!value.contains("<d:stCxn id=\"3\""));
    assert!(value.contains("<d:endCxn id=\"99\""));
    assert!(!value.contains("spid=\"4\""));
    assert!(value.contains("bldLst"));
    other_parts(&before, s.bytes());
    let deleted = s.bytes().to_vec();
    assert!(s.undo());
    assert_eq!(s.bytes(), before);
    assert!(s.redo());
    assert_eq!(s.bytes(), deleted);
    s.delete_shape(0, copy).unwrap();
    assert!(!slide_xml(s.bytes()).contains("bldLst"));
    assert!(!slide_xml(s.bytes()).contains("grpSp xmlns:g="));
    assert!(slide_xml(s.bytes()).contains("Outside connector"));
}
#[test]
fn opaque_child_references_and_ambiguous_descendant_ids_disable_only_unsafe_structural_actions() {
    let original = slide_xml(DECK);
    let opaque=original.replace("</p:sld>","<p:extLst><p:ext uri=\"test\"><z:ref xmlns:z=\"urn:test\" z:shapeId=\"2\"/></p:ext></p:extLst></p:sld>");
    let duplicate_id = original.replace(
        "id=\"99\" name=\"Outside target\"",
        "id=\"2\" name=\"Outside target\"",
    );
    for xml in [
        opaque,
        duplicate_id,
        original.replace("<jadeSp:cNvPr id=\"3\"", "<jadeSp:cNvPr id=\"2\""),
    ] {
        assert_ne!(xml, original);
        let bytes = edited(&xml);
        let info = editable_shapes(&bytes, 0)
            .unwrap()
            .into_iter()
            .find(|g| g.id == 50)
            .unwrap();
        assert!(info.can_move);
        assert!(!info.can_duplicate && !info.can_delete);
        let mut s = EditSession::open(&bytes).unwrap();
        assert!(s.duplicate_shape(0, 50).is_err());
        assert!(s.delete_shape(0, 50).is_err());
        assert_eq!(s.bytes(), bytes);
        assert!(!s.undo());
    }
}
#[test]
fn paired_group_branches_keep_logical_ids_and_creation_guids_consistent_and_reserved_refs_unallocated(
) {
    let original = slide_xml(DECK);
    let source = source_group(&original);
    let wrapper=format!("<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice xmlns:q=\"urn:test\" Requires=\"q\">{source}</mc:Choice><mc:Fallback>{source}</mc:Fallback></mc:AlternateContent>");
    let xml=original.replacen(source,&wrapper,1).replace("</p:sld>","<p:extLst><p:ext uri=\"test\"><z:ref xmlns:z=\"urn:test\" z:shapeId=\"4294967295\"/><z:ref xmlns:z=\"urn:test\" z:shapeId=\"5\"/></p:ext></p:extLst></p:sld>");
    let bytes = edited(&xml);
    let mut s = EditSession::open(&bytes).unwrap();
    let copy = s.duplicate_shape(0, 50).unwrap();
    assert_eq!(copy, 6);
    let value = slide_xml(s.bytes());
    assert_eq!(value.matches("<mc:AlternateContent").count(), 2);
    assert_eq!(value.matches("name=\"Outer group (copy)\"").count(), 2);
    let clones = groups(s.bytes());
    assert!(ids(&clones[0]).is_disjoint(&ids(&clones[1])));
    assert!(!ids(&clones[1]).contains(&5));
    assert!(!ids(&clones[1]).contains(&u32::MAX));
    assert_eq!(
        value
            .matches("{0980FF19-E7E7-493C-8D3E-15B2100EA940}")
            .count(),
        2
    );
    s.delete_shape(0, copy).unwrap();
    assert_eq!(groups(s.bytes()).len(), 1);
    assert_eq!(
        slide_xml(s.bytes()).matches("<mc:AlternateContent").count(),
        1
    );
}
#[test]
fn no_move_copies_keep_the_outer_position_and_selection_locks_block_structural_edits() {
    let original = slide_xml(DECK);
    let bytes = edited(&original.replacen(
        "<g:cNvGrpSpPr/>",
        "<g:cNvGrpSpPr><d:grpSpLocks noMove=\"1\"/></g:cNvGrpSpPr>",
        1,
    ));
    let mut s = EditSession::open(&bytes).unwrap();
    s.duplicate_shape(0, 50).unwrap();
    let groups = groups(s.bytes());
    assert_eq!(groups[0].transform, groups[1].transform);
    let bytes = edited(&original.replacen(
        "<g:cNvGrpSpPr/>",
        "<g:cNvGrpSpPr><d:grpSpLocks noSelect=\"1\"/></g:cNvGrpSpPr>",
        1,
    ));
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s.duplicate_shape(0, 50).is_err());
    assert!(s.delete_shape(0, 50).is_err());
    assert_eq!(s.bytes(), bytes);
    assert!(!s.undo());
}

#[test]
fn mutable_owned_graphic_parts_are_not_silently_shared_by_a_group_copy() {
    let original = slide_xml(DECK);
    let frame = r#"<g:graphicFrame><g:nvGraphicFramePr><g:cNvPr id="80" name="Unknown frame"/><g:cNvGraphicFramePr/><g:nvPr/></g:nvGraphicFramePr><g:xfrm><d:off x="0" y="0"/><d:ext cx="952500" cy="952500"/></g:xfrm><d:graphic><d:graphicData uri="urn:unknown"><x:owned xmlns:x="urn:unknown"/></d:graphicData></d:graphic></g:graphicFrame>"#;
    let at = original.rfind("</g:grpSp>").unwrap();
    let bytes = edited(&format!("{}{frame}{}", &original[..at], &original[at..]));
    let info = editable_shapes(&bytes, 0)
        .unwrap()
        .into_iter()
        .find(|g| g.id == 50)
        .unwrap();
    assert!(!info.can_duplicate);
    assert!(info.can_delete && info.can_move);
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s.duplicate_shape(0, 50).is_err());
    assert_eq!(s.bytes(), bytes);
    assert!(!s.undo());
    assert!(s.delete_shape(0, 50).unwrap());
    other_parts(&bytes, s.bytes());
}

#[test]
fn inline_table_children_copy_independently_without_changing_their_cells_or_local_transform() {
    let sample = OpcPackage::open(include_bytes!("../../../public/fixtures/sample-deck.pptx"))
        .unwrap()
        .required_part_str("ppt/slides/slide3.xml")
        .unwrap();
    let start = sample.find("<p:graphicFrame>").unwrap();
    let end =
        sample[start..].find("</p:graphicFrame>").unwrap() + start + "</p:graphicFrame>".len();
    let frame = &sample[start..end];
    let start = frame.find("<p:cNvPr id=\"").unwrap() + "<p:cNvPr id=\"".len();
    let end = frame[start..].find('"').unwrap() + start;
    let frame = format!("{}8000{}", &frame[..start], &frame[end..]);
    let original = slide_xml(DECK);
    let at = original.rfind("</g:grpSp>").unwrap();
    let bytes = edited(&format!("{}{frame}{}", &original[..at], &original[at..]));
    let mut s = EditSession::open(&bytes).unwrap();
    s.duplicate_shape(0, 50).unwrap();
    let groups = groups(s.bytes());
    let ShapeItem::Table(original) = groups[0].children.last().unwrap() else {
        panic!("table");
    };
    let ShapeItem::Table(copy) = groups[1].children.last().unwrap() else {
        panic!("table copy");
    };
    assert_ne!(original.id, copy.id);
    let mut copy = copy.clone();
    copy.id = original.id;
    assert_eq!(&copy, original);
    other_parts(&bytes, s.bytes());
    assert!(s.undo());
    assert_eq!(s.bytes(), bytes);
}
