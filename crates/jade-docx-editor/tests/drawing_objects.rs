//! Pictures, shapes and groups edited through the commands the object tools
//! use, checked in the package Word reads back.

mod common;

use common::*;
use jade_docx_editor::commands::format_image::{FormatImageCommand, ImageFormatSpec};
use jade_docx_editor::commands::transform_object::TransformObjectCommand;
use jade_docx_editor::word_document::{
    FloatingObjectKind, FloatingObjectTarget, FloatingObjectTransform,
};

fn picture_target() -> FloatingObjectTarget {
    FloatingObjectTarget {
        paragraph_index: 0,
        run_index: 0,
        kind: FloatingObjectKind::Image,
    }
}

const SHADOW: &str = r#"<a:ln w="38100"><a:solidFill><a:srgbClr val="C00000"/></a:solidFill></a:ln><a:effectLst><a:outerShdw blurRad="50800" dist="38100" dir="2700000" algn="tl" rotWithShape="0"><a:prstClr val="black"><a:alpha val="40000"/></a:prstClr></a:outerShdw></a:effectLst>"#;

/// Moving, resizing and re-wrapping a picture patch its frame; its outline,
/// shadow, picture locks and extension lists are still Word's.
#[test]
fn moving_a_picture_keeps_its_effects() {
    let body = format!(
        "<w:p>{}</w:p>",
        floating_picture(
            "Photo",
            SHADOW,
            r#"<a:extLst><a:ext uri="{28A0092B-C50C-407E-A947-70E740481C1C}"><a14:useLocalDpi val="0"/></a:ext></a:extLst>"#
        )
    );
    let mut session = Session::open(&body, &[("image1.png", png())]);
    session.run(Box::new(TransformObjectCommand::new(
        picture_target(),
        FloatingObjectTransform {
            x: 40.0,
            y: 30.0,
            width: 200.0,
            height: 100.0,
            rotation_deg: 0.0,
        },
    )));
    session.run(Box::new(FormatImageCommand::new(
        picture_target(),
        ImageFormatSpec {
            flip_horizontal: Some(true),
            ..Default::default()
        },
    )));
    let xml = session.xml();
    for kept in [
        r#"<a:ln w="38100">"#,
        r#"<a:outerShdw blurRad="50800""#,
        "a14:useLocalDpi",
        "a:graphicFrameLocks",
        r#"name="Photo""#,
        "wp:effectExtent",
    ] {
        assert!(xml.contains(kept), "lost {kept}: {xml}");
    }
    assert!(xml.contains(r#"<wp:extent cx="1905000" cy="952500"/>"#), "{xml}");
    assert!(xml.contains(r#"flipH="1""#), "{xml}");
    assert_eq!(xml.matches("<w:drawing>").count(), 1, "{xml}");

    let reopened = session.reopened();
    let image = reopened.paragraph(0).unwrap().runs[0].image.clone().unwrap();
    assert_eq!(image.width_px.map(f64::round), Some(200.0));
    assert!(image.layout.flip_horizontal);
}

/// A group dragged somewhere else is still the group: its shapes and their
/// coordinate space, not a picture of nothing.
#[test]
fn moving_a_group_keeps_its_shapes() {
    let body = r#"<w:p><w:r><w:drawing><wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" relativeHeight="251659264" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="1905000" cy="952500"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="5" name="Group 5"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><wpg:wgp><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1905000" cy="952500"/><a:chOff x="0" y="0"/><a:chExt cx="1905000" cy="952500"/></a:xfrm></wpg:grpSpPr><wps:wsp><wps:cNvPr id="6" name="Rectangle 6"/><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="952500" cy="952500"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp><wps:wsp><wps:cNvPr id="7" name="Oval 7"/><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="952500" y="0"/><a:ext cx="952500" cy="952500"/></a:xfrm><a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="ED7D31"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp></wpg:wgp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p>"#;
    let mut session = Session::open(body, &[]);
    assert!(session.document.paragraph(0).unwrap().runs[0]
        .image
        .as_ref()
        .unwrap()
        .shape_group
        .is_some());
    session.run(Box::new(TransformObjectCommand::new(
        picture_target(),
        FloatingObjectTransform {
            x: 10.0,
            y: 10.0,
            width: 400.0,
            height: 200.0,
            rotation_deg: 0.0,
        },
    )));
    let xml = session.xml();
    assert!(xml.contains("<wpg:wgp>"), "{xml}");
    assert!(xml.contains(r#"prst="ellipse""#), "{xml}");
    assert!(!xml.contains("<pic:pic"), "{xml}");
    // The group's frame is scaled; its children keep their own space.
    assert!(xml.contains(r#"<a:chExt cx="1905000" cy="952500"/>"#), "{xml}");
    assert!(xml.contains(r#"<a:ext cx="3810000" cy="1905000"/>"#), "{xml}");
    let reopened = session.reopened();
    let group = reopened.paragraph(0).unwrap().runs[0]
        .image
        .clone()
        .unwrap()
        .shape_group
        .unwrap();
    assert_eq!(group.children.len(), 2);
    session.undo();
    assert!(session.xml().contains(r#"<wp:extent cx="1905000" cy="952500"/>"#));
}
