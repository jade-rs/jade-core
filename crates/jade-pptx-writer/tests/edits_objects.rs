use jade_opc::OpcPackage;
use jade_pptx_writer::{
    arrange_shape, delete_shape, duplicate_shape, editable_shapes, EditSession, ObjectArrangement,
};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn custom(content: &str, tail: &str) -> Vec<u8> {
    let mut p = OpcPackage::open(DECK).unwrap();
    p.replace("ppt/slides/slide1.xml", format!(r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:z="urn:test"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{content}<p:extLst><p:ext uri="x"><z:tree/></p:ext></p:extLst></p:spTree></p:cSld>{tail}</p:sld>"#).into_bytes());
    p.write().unwrap()
}
fn shape(id: u32, name: &str, locks: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{name}"><a:extLst><a:ext uri="x"><z:payload/></a:ext></a:extLst></p:cNvPr><p:cNvSpPr>{locks}</p:cNvSpPr><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm rot='-60000' flipH='true' z:keep='&amp;'><a:off x='0010' y='020'/><a:ext cx='0300' cy='0400'/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr b="1"><a:hlinkClick r:id="rId99"/></a:rPr><a:t>Text 🦀</a:t></a:r></a:p></p:txBody></p:sp>"#
    )
}
fn xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap()
}
fn ids(bytes: &[u8]) -> Vec<u32> {
    editable_shapes(bytes, 0)
        .unwrap()
        .iter()
        .map(|o| o.id)
        .collect()
}
fn unchanged_other_parts(before: &[u8], after: &[u8]) {
    let before = OpcPackage::open(before).unwrap();
    let after = OpcPackage::open(after).unwrap();
    for part in before.part_names() {
        if part != "ppt/slides/slide1.xml" {
            assert_eq!(before.part(part), after.part(part), "{part}");
        }
    }
}
#[test]
fn duplicate_shape_preserves_original_styles_links_extensions_and_has_independent_history() {
    let original = shape(7, "A &amp; B", "");
    let bytes = custom(
        &format!("{original}{}", shape(9, "A &amp; B (copy)", "")),
        "",
    );
    let mut session = EditSession::open(&bytes).unwrap();
    let id = session.duplicate_shape(0, 7).unwrap();
    assert_eq!(id, 10);
    let copied = session.bytes().to_vec();
    assert_eq!(ids(&copied), [7, 9, 10]);
    assert!(xml(&copied).contains(&original));
    let objects = editable_shapes(&copied, 0).unwrap();
    let duplicate = &objects[2];
    assert_eq!(duplicate.name, "A & B (copy 2)");
    assert_eq!(duplicate.transform.unwrap().offset_emu, (127010, 127020));
    assert_eq!(duplicate.transform.unwrap().extent_emu, (300, 400));
    assert!(xml(&copied).contains("flipH='true' z:keep='&amp;'"));
    assert_eq!(xml(&copied).matches("<z:payload/>").count(), 3);
    assert_eq!(
        jade_pptx_writer::shape_text(&copied, 0, id).unwrap().text,
        "Text 🦀"
    );
    session.set_shape_text(0, id, "copy", false).unwrap();
    assert_eq!(
        jade_pptx_writer::shape_text(session.bytes(), 0, 7)
            .unwrap()
            .text,
        "Text 🦀"
    );
    assert!(session.undo());
    assert_eq!(session.bytes(), copied);
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
    assert!(session.redo());
    assert_eq!(session.bytes(), copied);
    unchanged_other_parts(&bytes, &copied);
}
#[test]
fn pictures_share_the_original_media_and_keep_crop_rotation_and_relationships() {
    let picture = r#"<p:pic><p:nvPicPr><p:cNvPr id="7" name="Image"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId99"/><a:srcRect l="25000" r="25000"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm rot="60000" flipV="1"><a:off x="100" y="200"/><a:ext cx="300" cy="400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#;
    let bytes = custom(picture, "");
    let (copy, id) = duplicate_shape(&bytes, 0, 7).unwrap();
    assert_eq!(id, 8);
    assert_eq!(xml(&copy).matches("r:embed=\"rId99\"").count(), 2);
    assert_eq!(
        xml(&copy)
            .matches("<a:srcRect l=\"25000\" r=\"25000\"/>")
            .count(),
        2
    );
    unchanged_other_parts(&bytes, &copy);
    let deleted = delete_shape(&copy, 0, id).unwrap();
    assert_eq!(ids(&deleted), [7]);
    unchanged_other_parts(&bytes, &deleted);
}
#[test]
fn arrange_moves_across_groups_and_preserves_header_extension_slots_and_noop_bytes() {
    let group = r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="50" name="Group"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:grpSp>"#;
    let bytes = custom(
        &format!(
            "{}<!--between-->{group}{}",
            shape(7, "one", ""),
            shape(9, "two", "")
        ),
        "",
    );
    let front = arrange_shape(&bytes, 0, 7, ObjectArrangement::Front).unwrap();
    assert_eq!(ids(&front), [50, 9, 7]);
    let group_info = editable_shapes(&front, 0)
        .unwrap()
        .into_iter()
        .find(|info| info.id == 50)
        .unwrap();
    assert!(!group_info.can_select && !group_info.can_arrange);
    let result = xml(&front);
    assert!(result.find("name=\"Group\"").unwrap() < result.find("name=\"two\"").unwrap());
    assert!(result.find("name=\"two\"").unwrap() < result.find("name=\"one\"").unwrap());
    assert!(result.find("name=\"one\"").unwrap() < result.find("<z:tree/>").unwrap());
    assert!(result.contains("<!--between-->"));
    assert!(result.contains(group));
    assert_eq!(
        arrange_shape(&front, 0, 7, ObjectArrangement::Front).unwrap(),
        front
    );
    assert_eq!(
        arrange_shape(&front, 0, 7, ObjectArrangement::Back).unwrap(),
        bytes
    );
    assert_eq!(
        arrange_shape(&bytes, 0, 7, ObjectArrangement::Forward).unwrap(),
        arrange_shape(&front, 0, 7, ObjectArrangement::Backward).unwrap()
    );
    unchanged_other_parts(&bytes, &front);
}
#[test]
fn deleting_detaches_only_matching_connector_endpoints_and_removes_empty_build_list() {
    let connector = r#"<p:cxnSp><p:nvCxnSpPr><p:cNvPr id="8" name="Connector"/><p:cNvCxnSpPr><a:stCxn id="7" idx="0"/><a:endCxn id="9" idx="1"/></p:cNvCxnSpPr><p:nvPr/></p:nvCxnSpPr><p:spPr><a:xfrm><a:off x="1" y="2"/><a:ext cx="3" cy="4"/></a:xfrm><a:prstGeom prst="line"><a:avLst/></a:prstGeom></p:spPr></p:cxnSp>"#;
    let bytes = custom(
        &format!("{}{connector}{}", shape(7, "one", ""), shape(9, "two", "")),
        r#"<p:timing><p:bldLst><p:bldP spid="7" grpId="0"/></p:bldLst></p:timing>"#,
    );
    let deleted = delete_shape(&bytes, 0, 7).unwrap();
    let result = xml(&deleted);
    assert!(!result.contains("<a:stCxn"));
    assert!(result.contains("<a:endCxn id=\"9\" idx=\"1\"/>"));
    assert!(!result.contains("bldLst"));
    assert_eq!(ids(&deleted), [8, 9]);
    let (copied, new_id) = duplicate_shape(&bytes, 0, 7).unwrap();
    assert_eq!(new_id, 10);
    assert!(xml(&copied).contains("spid=\"10\""));
    unchanged_other_parts(&bytes, &deleted);
}
#[test]
fn unsupported_timing_and_opaque_shape_references_are_transactionally_rejected() {
    for tail in [
        r#"<p:timing><p:tnLst><p:animEffect transition="in" filter="fade"><p:cBhvr><p:cTn id="7" dur="2000"/><p:tgtEl><p:spTgt spid="7"/></p:tgtEl></p:cBhvr></p:animEffect></p:tnLst></p:timing>"#,
        r#"<p:extLst><p:ext uri="x"><z:reference z:spid="7"/></p:ext></p:extLst>"#,
    ] {
        let bytes = custom(&shape(7, "one", ""), tail);
        let mut session = EditSession::open(&bytes).unwrap();
        let info = editable_shapes(&bytes, 0).unwrap().remove(0);
        assert!(!info.can_duplicate && !info.can_delete && info.can_arrange);
        assert!(session.duplicate_shape(0, 7).is_err());
        assert!(session.delete_shape(0, 7).is_err());
        assert_eq!(session.bytes(), bytes);
        assert!(!session.can_undo());
    }
}
#[test]
fn alternate_content_operations_copy_delete_and_arrange_the_whole_logical_object() {
    let branches = format!(
        r#"<mc:AlternateContent><mc:Choice Requires="z">{}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent>"#,
        shape(7, "one", ""),
        shape(7, "one", "")
    );
    let bytes = custom(&format!("{branches}{}", shape(9, "two", "")), "");
    let (copy, id) = duplicate_shape(&bytes, 0, 7).unwrap();
    assert_eq!(id, 10);
    let result = xml(&copy);
    assert_eq!(result.matches("id=\"10\"").count(), 2);
    assert_eq!(result.matches("x='127010'").count(), 2);
    assert_eq!(result.matches("<mc:AlternateContent").count(), 2);
    let back = arrange_shape(&bytes, 0, 9, ObjectArrangement::Back).unwrap();
    assert!(
        xml(&back).find("name=\"two\"").unwrap() < xml(&back).find("<mc:AlternateContent").unwrap()
    );
    let deleted = delete_shape(&bytes, 0, 7).unwrap();
    assert!(!xml(&deleted).contains("AlternateContent"));
    assert_eq!(ids(&deleted), [9]);
    let multi = custom(
        &format!(
            r#"<mc:AlternateContent><mc:Fallback>{}{}</mc:Fallback></mc:AlternateContent>"#,
            shape(7, "one", ""),
            shape(9, "two", "")
        ),
        "",
    );
    let info = editable_shapes(&multi, 0).unwrap();
    assert!(info
        .iter()
        .all(|i| !i.can_duplicate && !i.can_delete && !i.can_arrange));
    assert!(duplicate_shape(&multi, 0, 7).is_err());
}
#[test]
fn strict_namespaces_inherited_geometry_and_selection_move_locks_are_retained() {
    // The sample title already inherits its transform from the layout.
    let bytes = DECK.to_vec();
    let before = editable_shapes(&bytes, 0)
        .unwrap()
        .into_iter()
        .find(|i| i.id == 2)
        .unwrap()
        .transform
        .unwrap();
    let (copy, id) = duplicate_shape(&bytes, 0, 2).unwrap();
    let copied = editable_shapes(&copy, 0)
        .unwrap()
        .into_iter()
        .find(|i| i.id == id)
        .unwrap()
        .transform
        .unwrap();
    assert_eq!(copied.extent_emu, before.extent_emu);
    assert_eq!(
        copied.offset_emu,
        (before.offset_emu.0 + 127000, before.offset_emu.1 + 127000)
    );
    let strict = custom(&shape(7, "one", "<a:spLocks noMove=\"1\"/>"), "");
    let mut package = OpcPackage::open(&strict).unwrap();
    package.replace(
        "ppt/slides/slide1.xml",
        xml(&strict)
            .replace(
                "http://schemas.openxmlformats.org/drawingml/2006/main",
                "http://purl.oclc.org/ooxml/drawingml/main",
            )
            .replace(
                "http://schemas.openxmlformats.org/presentationml/2006/main",
                "http://purl.oclc.org/ooxml/presentationml/main",
            )
            .into_bytes(),
    );
    let strict = package.write().unwrap();
    let (copy, id) = duplicate_shape(&strict, 0, 7).unwrap();
    assert_eq!(
        editable_shapes(&copy, 0)
            .unwrap()
            .into_iter()
            .find(|i| i.id == id)
            .unwrap()
            .transform
            .unwrap()
            .offset_emu,
        (10, 20)
    );
    let locked = custom(&shape(7, "one", "<a:spLocks noSelect=\"true\"/>"), "");
    assert!(duplicate_shape(&locked, 0, 7).is_err());
    assert!(delete_shape(&locked, 0, 7).is_err());
    assert!(arrange_shape(&locked, 0, 7, ObjectArrangement::Front).is_err());
}
#[test]
fn duplicate_regenerates_office_creation_ids_and_allocates_an_unused_numeric_id_at_the_limit() {
    let source=shape(u32::MAX,"one","").replace("<z:payload/>",r#"<z:payload/><a16:creationId xmlns:a16="http://schemas.microsoft.com/office/drawing/2014/main" id="{0980FF19-E7E7-493C-8D3E-15B2100EA940}"/>"#);
    let bytes = custom(&source, "");
    let (copy, id) = duplicate_shape(&bytes, 0, u32::MAX).unwrap();
    assert_eq!(id, 2);
    let result = xml(&copy);
    assert_eq!(
        result
            .matches("id=\"{0980FF19-E7E7-493C-8D3E-15B2100EA940}\"")
            .count(),
        1
    );
    assert_eq!(result.matches("a16:creationId").count(), 2);
    assert!(result.contains(&source));
}

#[test]
fn ids_colliding_with_group_children_are_not_structurally_editable() {
    let grouped = format!(
        r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="50" name="Group"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}</p:grpSp>"#,
        shape(7, "nested", "")
    );
    let bytes = custom(&format!("{}{grouped}", shape(7, "root", "")), "");
    let info = editable_shapes(&bytes, 0).unwrap().remove(0);
    assert!(!info.can_select && !info.can_duplicate && !info.can_delete && !info.can_arrange);
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session.duplicate_shape(0, 7).is_err());
    assert!(session.delete_shape(0, 7).is_err());
    assert!(session
        .arrange_shape(0, 7, ObjectArrangement::Front)
        .is_err());
    let moved = jade_pptx_reader::Transform {
        offset_emu: (100, 200),
        ..info.transform.unwrap()
    };
    assert!(session.set_shape_transform(0, 7, moved).is_err());
    assert_eq!(session.bytes(), bytes);
    assert!(!session.can_undo());
}
