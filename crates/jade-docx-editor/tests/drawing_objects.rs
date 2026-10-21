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

/// An edit of the drawing's XML is read back as the object: the model draws
/// what the package now says, the save carries it, and undo takes it back.
#[test]
fn a_drawing_edit_is_read_back_and_undone() {
    use jade_docx_editor::commands::edit_drawing::edit_drawing_command;
    use jade_docx_editor::document::BlipEffect;
    use jade_docx_editor::drawing_xml::{child_mut, descendant_mut, put_shape_property};
    use jade_docx_editor::word_document::objects::parse_element;

    let body = format!("<w:p>{}</w:p>", floating_picture("Photo", "", ""));
    let mut session = Session::open(&body, &[("image1.png", png())]);
    session.run(Box::new(edit_drawing_command(
        "picture_effects",
        picture_target(),
        |_document, drawing| {
            let blip = descendant_mut(drawing, "a:blip").ok_or("no blip")?;
            blip.children.push(jade_docx_editor::docx_xml::XmlNode::Element(
                parse_element(r#"<a:grayscl xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"/>"#)?,
            ));
            let sp_pr = jade_docx_editor::drawing_xml::shape_properties_mut(drawing).ok_or("no spPr")?;
            let effects = parse_element(
                r#"<a:effectLst xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:reflection blurRad="6350" stA="50000" endA="300" endPos="55000" dir="5400000" sy="-100000" algn="bl" rotWithShape="0"/></a:effectLst>"#,
            )?;
            put_shape_property(sp_pr, effects, &["a:effectLst"]);
            let _ = child_mut(sp_pr, "a:xfrm");
            Ok(())
        },
    )));
    let image = session.document.paragraph(0).unwrap().runs[0]
        .image
        .clone()
        .unwrap();
    assert_eq!(image.format().blip_effects, vec![BlipEffect::Grayscale]);
    let reflection = image.format().reflection.clone().expect("reflection read back");
    assert!((reflection.start_opacity - 0.5).abs() < 1e-9);
    assert_eq!(reflection.scale_y, -1.0);
    assert!(image.data_uri.is_some(), "the picture is still resolved");
    assert_eq!(image.layout.options().name.as_deref(), Some("Photo"));

    let xml = session.xml();
    assert!(xml.contains("<a:grayscl"), "{xml}");
    assert!(xml.contains("<a:reflection"), "{xml}");
    // Moving it afterwards keeps the edit.
    session.run(Box::new(TransformObjectCommand::new(
        picture_target(),
        FloatingObjectTransform {
            x: 10.0,
            y: 10.0,
            width: 100.0,
            height: 50.0,
            rotation_deg: 0.0,
        },
    )));
    assert!(session.xml().contains("<a:reflection"));
    session.undo();
    session.undo();
    let image = session.document.paragraph(0).unwrap().runs[0]
        .image
        .clone()
        .unwrap();
    assert!(image.format().blip_effects.is_empty());
    assert!(image.format().reflection.is_none());
    assert!(!session.xml().contains("<a:grayscl"));
}

/// What a drawing carries beyond its frame is read: the anchor's options,
/// the name, a freeform's path, a connector's ends, WordArt's warp, a text
/// box chain.
#[test]
fn drawing_extras_are_read() {
    let shape = |name: &str, id: u32, sp_pr: &str, extra: &str, body_pr: &str| {
        format!(
            r#"<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="5" behindDoc="0" locked="1" layoutInCell="0" allowOverlap="0"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>914400</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>914400</wp:posOffset></wp:positionV><wp:extent cx="952500" cy="952500"/><wp:wrapNone/><wp:docPr id="{id}" name="{name}" hidden="1"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp>{extra}<wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="952500" cy="952500"/></a:xfrm>{sp_pr}</wps:spPr>{body_pr}</wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#
        )
    };
    let freeform = shape(
        "Freeform 1",
        3,
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l="0" t="0" r="r" b="b"/><a:pathLst><a:path w="100" h="100"><a:moveTo><a:pt x="0" y="100"/></a:moveTo><a:lnTo><a:pt x="50" y="0"/></a:lnTo><a:cubicBezTo><a:pt x="60" y="10"/><a:pt x="80" y="10"/><a:pt x="100" y="100"/></a:cubicBezTo><a:close/></a:path></a:pathLst></a:custGeom>"#,
        "<wps:cNvSpPr/>",
        "<wps:bodyPr/>",
    );
    let connector = shape(
        "Connector 2",
        4,
        r#"<a:prstGeom prst="straightConnector1"><a:avLst/></a:prstGeom>"#,
        r#"<wps:cNvCnPr><a:stCxn id="3" idx="2"/><a:endCxn id="5" idx="0"/></wps:cNvCnPr>"#,
        "<wps:bodyPr/>",
    );
    let wordart = shape(
        "WordArt 3",
        5,
        r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>"#,
        r#"<wps:cNvSpPr txBox="1"/>"#,
        r#"<wps:txbx id="7"><w:txbxContent><w:p><w:r><w:t>Arch</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr wrap="none"><a:prstTxWarp prst="textArchUp"><a:avLst><a:gd name="adj" fmla="val 10800000"/></a:avLst></a:prstTxWarp><a:spAutoFit/></wps:bodyPr>"#,
    );
    let body = format!("<w:p>{freeform}{connector}{wordart}</w:p>");
    let session = Session::open(&body, &[]);
    let runs = &session.document.paragraph(0).unwrap().runs;
    let boxes: Vec<_> = runs.iter().filter_map(|run| run.text_box.clone()).collect();
    assert_eq!(boxes.len(), 3);
    let layout = boxes[0].layout.options();
    assert_eq!(layout.name.as_deref(), Some("Freeform 1"));
    assert_eq!(layout.object_id, Some(3));
    assert!(layout.hidden && layout.locked);
    assert_eq!(layout.allow_overlap, Some(false));
    assert_eq!(layout.layout_in_cell, Some(false));
    let geometry = boxes[0].extras().custom_geometry.as_ref().expect("freeform path");
    assert_eq!(geometry.paths.len(), 1);
    assert_eq!(geometry.paths[0].commands.len(), 4);
    let ends = boxes[1].extras().connector.expect("connector ends");
    assert_eq!(ends.start.map(|end| (end.id, end.index)), Some((3, 2)));
    assert_eq!(ends.end.map(|end| (end.id, end.index)), Some((5, 0)));
    let warp = boxes[2].extras().text_warp.as_ref().expect("warp");
    assert_eq!(warp.preset, "textArchUp");
    assert_eq!(warp.adjustments, vec![("adj".to_string(), 10_800_000)]);
    assert_eq!(boxes[2].extras().link.map(|link| (link.id, link.sequence)), Some((7, 0)));
}
