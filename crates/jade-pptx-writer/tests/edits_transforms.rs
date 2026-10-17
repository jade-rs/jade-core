use jade_opc::OpcPackage;
use jade_pptx_reader::Transform;
use jade_pptx_writer::{editable_shapes, set_shape_transform, EditSession};

const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn transform() -> Transform {
    Transform {
        offset_emu: (100, 200),
        extent_emu: (300, 400),
        rotation_60000ths: 5400000,
        flip_h: true,
        flip_v: false,
    }
}
fn custom(shape: &str) -> Vec<u8> {
    let mut p = OpcPackage::open(DECK).unwrap();
    p.replace("ppt/slides/slide1.xml", format!("<p:sld xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:z=\"urn:extension\"><p:cSld><p:spTree>{shape}</p:spTree></p:cSld></p:sld>").into_bytes());
    p.write().unwrap()
}
fn shape(properties: &str, locks: &str) -> String {
    format!("<p:sp><p:nvSpPr><p:cNvPr id=\"7\" name=\"Shape\"/><p:cNvSpPr>{locks}</p:cNvSpPr></p:nvSpPr>{properties}<p:txBody><a:bodyPr/><a:p><a:r><a:rPr b=\"1\"/><a:t>keep text</a:t></a:r></a:p></p:txBody><p:extLst><z:payload/></p:extLst></p:sp>")
}

#[test]
fn changes_only_requested_coordinate_values_and_retains_lexical_attributes() {
    let source = shape("<p:spPr><a:xfrm rot='-60000' flipH='true' z:keep='&amp;'><a:off x='0010' y='020'/><a:ext cx='0300' cy='0400'/><z:extra/></a:xfrm><a:prstGeom prst='ellipse'/><a:solidFill><a:srgbClr val='FF0000'/></a:solidFill></p:spPr>", "");
    let bytes = custom(&source);
    let before = editable_shapes(&bytes, 0)
        .unwrap()
        .remove(0)
        .transform
        .unwrap();
    let after = Transform {
        offset_emu: (40, 20),
        extent_emu: (300, 500),
        ..before
    };
    let output = set_shape_transform(&bytes, 0, 7, after).unwrap();
    let original = OpcPackage::open(&bytes).unwrap();
    let saved = OpcPackage::open(&output).unwrap();
    for part in original.part_names() {
        if part == "ppt/slides/slide1.xml" {
            let expected = original
                .required_part_str(part)
                .unwrap()
                .replace("x='0010'", "x='40'")
                .replace("cy='0400'", "cy='500'");
            assert_eq!(saved.required_part_str(part).unwrap(), expected);
        } else {
            assert_eq!(original.part(part), saved.part(part), "{part}");
        }
    }
    assert_eq!(
        editable_shapes(&output, 0).unwrap()[0].transform,
        Some(after)
    );
    assert_eq!(set_shape_transform(&output, 0, 7, after).unwrap(), output);
}

#[test]
fn materializes_placeholder_geometry_and_missing_shape_properties_in_schema_order() {
    for properties in [
        "<p:spPr/>",
        "<p:spPr z:keep='yes'/>",
        "<p:spPr><a:prstGeom prst='rect'/></p:spPr>",
        "",
    ] {
        let bytes = custom(&shape(properties, ""));
        let saved = set_shape_transform(&bytes, 0, 7, transform()).unwrap();
        assert_eq!(
            editable_shapes(&saved, 0).unwrap()[0].transform,
            Some(transform())
        );
        let xml = OpcPackage::open(&saved)
            .unwrap()
            .required_part_str("ppt/slides/slide1.xml")
            .unwrap();
        assert!(xml.contains("keep text"));
        assert!(xml.contains("<z:payload/>"));
        if properties.contains("z:keep") {
            assert!(xml.contains("z:keep='yes'"));
        }
        if properties.contains("prstGeom") {
            assert!(xml.find(":xfrm").unwrap() < xml.find("prstGeom").unwrap());
        }
    }
}

#[test]
fn updates_picture_geometry_and_preserves_crops_and_relationships() {
    let bytes = custom("<p:pic><p:nvPicPr><p:cNvPr id='9' name='Picture'/></p:nvPicPr><p:blipFill><a:blip xmlns:r='http://schemas.openxmlformats.org/officeDocument/2006/relationships' r:embed='rId99'/><a:srcRect l='25000'/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm/></p:spPr></p:pic>");
    let saved = set_shape_transform(&bytes, 0, 9, transform()).unwrap();
    let before = OpcPackage::open(&bytes)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap();
    let after = OpcPackage::open(&saved)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap();
    assert_eq!(
        before
            .split("<p:blipFill>")
            .nth(1)
            .unwrap()
            .split("</p:blipFill>")
            .next(),
        after
            .split("<p:blipFill>")
            .nth(1)
            .unwrap()
            .split("</p:blipFill>")
            .next()
    );
    assert_eq!(
        editable_shapes(&saved, 0).unwrap()[0].transform,
        Some(transform())
    );
}

#[test]
fn rejects_locked_invalid_group_child_and_duplicate_shape_edits_atomically() {
    let bytes = custom(&shape(
        "<p:spPr><a:xfrm><a:off x='100' y='200'/><a:ext cx='300' cy='400'/></a:xfrm></p:spPr>",
        "<a:spLocks noMove='true' noResize='1' noRot='1'/>",
    ));
    let mut session = EditSession::open(&bytes).unwrap();
    let info = editable_shapes(&bytes, 0).unwrap().remove(0);
    assert!(!info.can_move && !info.can_resize && !info.can_rotate);
    assert!(session.set_shape_transform(0, 7, transform()).is_err());
    assert_eq!(session.bytes(), bytes);
    assert!(!session.is_dirty() && !session.can_undo());
    for invalid in [
        Transform {
            offset_emu: (i64::MAX, 0),
            ..transform()
        },
        Transform {
            extent_emu: (-1, 5),
            ..transform()
        },
    ] {
        assert!(set_shape_transform(DECK, 0, 2, invalid).is_err());
    }
    let grouped = custom(&format!("<p:grpSp>{}</p:grpSp>", shape("<p:spPr/>", "")));
    assert!(editable_shapes(&grouped, 0).unwrap().is_empty());
    assert!(set_shape_transform(&grouped, 0, 7, transform()).is_err());
    let duplicate = custom(&(shape("<p:spPr/>", "") + &shape("<p:spPr/>", "")));
    assert!(editable_shapes(&duplicate, 0).unwrap().is_empty());
    assert!(set_shape_transform(&duplicate, 0, 7, transform()).is_err());
}

#[test]
fn transform_undo_redo_restores_original_bytes_and_saved_geometry() {
    let mut session = EditSession::open(DECK).unwrap();
    session.set_shape_transform(0, 2, transform()).unwrap();
    assert!(session.undo());
    assert_eq!(session.bytes(), DECK);
    assert!(session.redo());
    assert_eq!(
        editable_shapes(session.bytes(), 0).unwrap()[0].transform,
        Some(transform())
    );
}

#[test]
fn aspect_ratio_lock_allows_proportional_resize_and_rejects_distortion() {
    let bytes = custom(&shape(
        "<p:spPr><a:xfrm><a:off x='0' y='0'/><a:ext cx='300' cy='200'/></a:xfrm></p:spPr>",
        "<a:spLocks noChangeAspect='1'/>",
    ));
    let info = editable_shapes(&bytes, 0).unwrap().remove(0);
    assert!(info.lock_aspect_ratio);
    let before = info.transform.unwrap();
    assert!(set_shape_transform(
        &bytes,
        0,
        7,
        Transform {
            extent_emu: (600, 400),
            ..before
        }
    )
    .is_ok());
    assert!(set_shape_transform(
        &bytes,
        0,
        7,
        Transform {
            extent_emu: (600, 200),
            ..before
        }
    )
    .is_err());
    let huge = custom(&shape(
        &format!(
            "<p:spPr><a:xfrm><a:off x='0' y='0'/><a:ext cx='{}' cy='{}'/></a:xfrm></p:spPr>",
            i64::MAX,
            i64::MAX
        ),
        "<a:spLocks noChangeAspect='1'/>",
    ));
    let result = std::panic::catch_unwind(|| set_shape_transform(&huge, 0, 7, transform()));
    assert!(
        result.is_ok(),
        "invalid authored extents must not overflow lock validation"
    );
}

#[test]
fn inherited_placeholder_transform_uses_layout_rules_and_preserves_resize_locks() {
    let source = shape("<p:spPr/>", "<a:spLocks noResize='1' noChangeAspect='1'/>").replace(
        "</p:nvSpPr>",
        "<p:nvPr><p:ph type='title'/></p:nvPr></p:nvSpPr>",
    );
    let bytes = custom(&source);
    let info = editable_shapes(&bytes, 0).unwrap().remove(0);
    let before = info.transform.expect("the layout/master supplies geometry");
    assert!(info.can_move && !info.can_resize);
    assert_eq!(set_shape_transform(&bytes, 0, 7, before).unwrap(), bytes);
    let moved = Transform {
        offset_emu: (before.offset_emu.0 + 9525, before.offset_emu.1),
        ..before
    };
    let output = set_shape_transform(&bytes, 0, 7, moved).unwrap();
    assert_eq!(
        editable_shapes(&output, 0).unwrap()[0].transform,
        Some(moved)
    );
    assert!(set_shape_transform(
        &bytes,
        0,
        7,
        Transform {
            extent_emu: (before.extent_emu.0 + 9525, before.extent_emu.1),
            ..before
        }
    )
    .is_err());
}
