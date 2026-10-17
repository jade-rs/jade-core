use jade_opc::OpcPackage;
use jade_pptx_writer::{editable_shapes, EditSession};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/grouped-deck.pptx");
fn xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap()
}
#[test]
fn group_transform_keeps_all_child_xml_and_parts_and_is_one_undo() {
    let info = editable_shapes(DECK, 0).unwrap();
    let group = info.iter().find(|s| s.id == 50).unwrap();
    assert_eq!(group.kind, "group");
    assert!(group.can_move && group.can_resize && group.can_rotate && group.can_arrange);
    assert!(group.can_duplicate && group.can_delete);
    assert!(!info.iter().any(|s| s.id == 51));
    let mut after = group.transform.unwrap();
    after.offset_emu = (400 * 9525, 250 * 9525);
    after.extent_emu = (800 * 9525, 300 * 9525);
    after.rotation_60000ths = 90 * 60000;
    after.flip_h = true;
    let mut s = EditSession::open(DECK).unwrap();
    assert!(!s
        .set_shape_transform(0, 50, group.transform.unwrap())
        .unwrap());
    assert_eq!(s.bytes(), DECK);
    assert!(s.set_shape_transform(0, 50, after).unwrap());
    let old = xml(DECK);
    let new = xml(s.bytes());
    let children = |value: String| {
        value[value.match_indices("<g:grpSp xmlns:g=").nth(1).unwrap().0..].to_owned()
    };
    assert_eq!(children(old), children(new));
    let before = OpcPackage::open(DECK).unwrap();
    let saved = OpcPackage::open(s.bytes()).unwrap();
    for part in before
        .part_names()
        .iter()
        .filter(|p| p.as_str() != "ppt/slides/slide1.xml")
    {
        assert_eq!(before.part(part), saved.part(part), "{part}");
    }
    let bytes = s.bytes().to_vec();
    assert!(s.undo());
    assert_eq!(s.bytes(), DECK);
    assert!(!s.undo());
    assert!(s.redo());
    assert_eq!(s.bytes(), bytes);
}
#[test]
fn group_locks_degenerate_spaces_and_duplicate_group_ids_reject_without_history() {
    let mut s = EditSession::open(DECK).unwrap();
    let mut collapsed = editable_shapes(DECK, 0)
        .unwrap()
        .into_iter()
        .find(|s| s.id == 50)
        .unwrap()
        .transform
        .unwrap();
    collapsed.extent_emu.0 = 0;
    assert!(s.set_shape_transform(0, 50, collapsed).is_err());
    assert_eq!(s.bytes(), DECK);
    assert!(!s.undo());
    let original = xml(DECK);
    for changed in [
        original.replacen(
            "<g:cNvGrpSpPr/>",
            "<g:cNvGrpSpPr><d:grpSpLocks noMove=\"true\"/></g:cNvGrpSpPr>",
            1,
        ),
        original.replacen(
            "<g:cNvGrpSpPr/>",
            "<g:cNvGrpSpPr><d:grpSpLocks noSelect=\"1\"/></g:cNvGrpSpPr>",
            1,
        ),
        original.replacen("<d:chExt cx=\"3810000\"", "<d:chExt cx=\"0\"", 1),
        original.replace("id=\"51\"", "id=\"50\""),
    ] {
        let mut package = OpcPackage::open(DECK).unwrap();
        package.replace("ppt/slides/slide1.xml", changed.into_bytes());
        let bytes = package.write().unwrap();
        let mut session = EditSession::open(&bytes).unwrap();
        let mut t = editable_shapes(DECK, 0)
            .unwrap()
            .into_iter()
            .find(|s| s.id == 50)
            .unwrap()
            .transform
            .unwrap();
        t.offset_emu.0 += 9525;
        assert!(session.set_shape_transform(0, 50, t).is_err());
        assert_eq!(session.bytes(), bytes);
        assert!(!session.undo());
    }
}

#[test]
fn compatible_alternate_group_branches_update_together_and_keep_child_spaces() {
    let xml = xml(DECK);
    let start = xml.find("<g:grpSp xmlns:g=").unwrap();
    let end = xml.rfind("</g:grpSp>").unwrap() + "</g:grpSp>".len();
    let group = &xml[start..end];
    let wrapper=format!("<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice xmlns:q=\"urn:test\" Requires=\"q\">{group}</mc:Choice><mc:Fallback>{group}</mc:Fallback></mc:AlternateContent>");
    let mut p = OpcPackage::open(DECK).unwrap();
    p.replace(
        "ppt/slides/slide1.xml",
        format!("{}{}{}", &xml[..start], wrapper, &xml[end..]).into_bytes(),
    );
    let bytes = p.write().unwrap();
    let mut s = EditSession::open(&bytes).unwrap();
    let mut t = editable_shapes(&bytes, 0)
        .unwrap()
        .into_iter()
        .find(|s| s.id == 50)
        .unwrap()
        .transform
        .unwrap();
    t.rotation_60000ths = 5400000;
    t.flip_h = true;
    assert!(s.set_shape_transform(0, 50, t).unwrap());
    let saved = OpcPackage::open(s.bytes())
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap();
    assert_eq!(saved.matches("rot=\"5400000\"").count(), 2);
    assert_eq!(
        saved
            .matches("<d:chExt cx=\"3810000\" cy=\"1905000\"/>")
            .count(),
        2
    );
    assert!(s.undo());
    assert_eq!(s.bytes(), bytes);
}
