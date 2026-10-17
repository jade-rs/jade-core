//! The slide body: shapes, pictures, groups, fills, and text.
//!
//! These tests drive the second slice of pptx support — parsing what is *on*
//! a slide, not just which slides exist. They exercise `read_deck` end to end
//! on an in-memory package, the way the package-level tests already do.

use std::io::Write;

use jade_pptx_reader::{read_deck, Color, Fill, ShapeItem, TextAlign};

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const PRESENTATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>
  <p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst>
  <p:sldSz cx="12192000" cy="6858000"/>
</p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#;

const SLIDE_MASTER: &str = r#"<p:sldMaster xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree/></p:cSld></p:sldMaster>"#;

const SLIDE_MASTER_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/>
</Relationships>"#;

const THEME: &str = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office">
  <a:themeElements>
    <a:clrScheme name="Office">
      <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
      <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
      <a:dk2><a:srgbClr val="44546A"/></a:dk2>
      <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
      <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
      <a:accent3><a:srgbClr val="A5A5A5"/></a:accent3>
      <a:accent4><a:srgbClr val="FFC000"/></a:accent4>
      <a:accent5><a:srgbClr val="5B9BD5"/></a:accent5>
      <a:accent6><a:srgbClr val="70AD47"/></a:accent6>
      <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
      <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
    </a:clrScheme>
  </a:themeElements>
</a:theme>"#;

const SLIDE_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
</Relationships>"#;

const SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr/>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr>
          <a:xfrm rot="5400000" flipH="1"><a:off x="914400" y="457200"/><a:ext cx="1828800" cy="914400"/></a:xfrm>
          <a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom>
          <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
          <a:ln w="25400"><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></a:ln>
        </p:spPr>
        <p:txBody>
          <a:bodyPr/>
          <a:p>
            <a:pPr algn="ctr" lvl="1"/>
            <a:r>
              <a:rPr lang="en-US" sz="3200" b="1" i="1"><a:solidFill><a:srgbClr val="112233"/></a:solidFill><a:latin typeface="Meiryo"/></a:rPr>
              <a:t>Hello</a:t>
            </a:r>
            <a:br/>
            <a:r><a:rPr sz="1800"/><a:t>World</a:t></a:r>
          </a:p>
          <a:p><a:r><a:t>Second</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="5" name="NoFill 4"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr>
          <a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
          <a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom>
          <a:noFill/>
        </p:spPr>
      </p:sp>
      <p:pic>
        <p:nvPicPr><p:cNvPr id="3" name="Picture 2"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
        <p:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>
        <p:spPr><a:xfrm><a:off x="5000000" y="1000000"/><a:ext cx="2000000" cy="1500000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
      </p:pic>
      <p:grpSp>
        <p:nvGrpSpPr><p:cNvPr id="4" name="Group 3"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
        <p:grpSpPr>
          <a:xfrm><a:off x="1000000" y="2000000"/><a:ext cx="2000000" cy="2000000"/><a:chOff x="0" y="0"/><a:chExt cx="1000000" cy="1000000"/></a:xfrm>
        </p:grpSpPr>
        <p:sp>
          <p:nvSpPr><p:cNvPr id="6" name="Grouped 5"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
          <p:spPr>
            <a:xfrm><a:off x="100000" y="100000"/><a:ext cx="500000" cy="500000"/></a:xfrm>
            <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
            <a:solidFill><a:schemeClr val="accent2"/></a:solidFill>
          </p:spPr>
        </p:sp>
      </p:grpSp>
    </p:spTree>
  </p:cSld>
</p:sld>"#;

fn deck_bytes() -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slideMasters/slideMaster1.xml", SLIDE_MASTER),
            (
                "ppt/slideMasters/_rels/slideMaster1.xml.rels",
                SLIDE_MASTER_RELS,
            ),
            ("ppt/theme/theme1.xml", THEME),
            ("ppt/slides/slide1.xml", SLIDE),
            ("ppt/slides/_rels/slide1.xml.rels", SLIDE_RELS),
            ("ppt/media/image1.png", "not-really-a-png"),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    buffer
}

fn first_shape(deck: &jade_pptx_reader::Deck) -> &jade_pptx_reader::Shape {
    match &deck.slides[0].content.shapes[0] {
        ShapeItem::Shape(shape) => shape,
        other => panic!("expected a shape first, got {other:?}"),
    }
}

#[test]
fn a_shape_comes_back_with_its_transform_geometry_and_fill() {
    let deck = read_deck(&deck_bytes()).expect("read");
    let shape = first_shape(&deck);

    assert_eq!(shape.name, "Title 1");
    let transform = shape.transform.expect("transform");
    assert_eq!(transform.offset_emu, (914_400, 457_200));
    assert_eq!(transform.extent_emu, (1_828_800, 914_400));
    // rot is in 60000ths of a degree; 5400000 is a quarter turn.
    assert_eq!(transform.rotation_60000ths, 5_400_000);
    assert!(transform.flip_h);
    assert!(!transform.flip_v);

    assert_eq!(shape.preset_geometry.as_deref(), Some("roundRect"));
    assert_eq!(
        shape.fill,
        Some(Fill::Solid(Color::Srgb("FF0000".to_string())))
    );

    let outline = shape.outline.as_ref().expect("outline");
    assert_eq!(outline.width_emu, Some(25_400));
    assert_eq!(outline.color, Some(Color::Scheme("accent1".to_string())));
}

#[test]
fn text_runs_carry_size_weight_color_and_breaks() {
    let deck = read_deck(&deck_bytes()).expect("read");
    let shape = first_shape(&deck);
    let text = shape.text.as_ref().expect("text body");

    assert_eq!(text.paragraphs.len(), 2);
    let first = &text.paragraphs[0];
    assert_eq!(first.align, Some(TextAlign::Center));
    assert_eq!(first.level, 1);

    // Run, explicit break, run.
    assert_eq!(first.runs.len(), 3);
    assert_eq!(first.runs[0].text, "Hello");
    // sz is in hundredths of a point.
    assert_eq!(first.runs[0].size_pt, Some(32.0));
    assert!(first.runs[0].bold);
    assert!(first.runs[0].italic);
    assert_eq!(first.runs[0].color, Some(Color::Srgb("112233".to_string())));
    assert_eq!(first.runs[0].font.as_deref(), Some("Meiryo"));

    assert_eq!(first.runs[1].text, "\n");
    assert_eq!(first.runs[2].text, "World");
    assert_eq!(first.runs[2].size_pt, Some(18.0));
    assert!(!first.runs[2].bold);

    assert_eq!(text.paragraphs[1].runs[0].text, "Second");
}

#[test]
fn the_title_placeholder_is_identified() {
    let deck = read_deck(&deck_bytes()).expect("read");
    let shape = first_shape(&deck);
    assert_eq!(
        shape.placeholder.as_ref().map(|ph| ph.kind.as_str()),
        Some("title")
    );
}

#[test]
fn an_explicit_no_fill_is_distinguished_from_an_absent_fill() {
    let deck = read_deck(&deck_bytes()).expect("read");
    let ShapeItem::Shape(no_fill) = &deck.slides[0].content.shapes[1] else {
        panic!("expected shape");
    };
    assert_eq!(no_fill.fill, Some(Fill::None));
    assert_eq!(no_fill.preset_geometry.as_deref(), Some("ellipse"));
}

#[test]
fn a_picture_resolves_its_image_part_through_the_slide_rels() {
    let deck = read_deck(&deck_bytes()).expect("read");
    let ShapeItem::Picture(picture) = &deck.slides[0].content.shapes[2] else {
        panic!("expected picture");
    };
    assert_eq!(picture.name, "Picture 2");
    assert_eq!(picture.image_part.as_deref(), Some("ppt/media/image1.png"));
    let transform = picture.transform.expect("transform");
    assert_eq!(transform.offset_emu, (5_000_000, 1_000_000));
}

#[test]
fn a_group_keeps_its_children_and_its_child_coordinate_space() {
    let deck = read_deck(&deck_bytes()).expect("read");
    let ShapeItem::Group(group) = &deck.slides[0].content.shapes[3] else {
        panic!("expected group");
    };
    assert_eq!(group.name, "Group 3");
    let transform = group.transform.as_ref().expect("group transform");
    assert_eq!(transform.outer.offset_emu, (1_000_000, 2_000_000));
    assert_eq!(transform.child_offset_emu, (0, 0));
    assert_eq!(transform.child_extent_emu, (1_000_000, 1_000_000));

    assert_eq!(group.children.len(), 1);
    let ShapeItem::Shape(child) = &group.children[0] else {
        panic!("expected child shape");
    };
    assert_eq!(child.name, "Grouped 5");
    assert_eq!(
        child.fill,
        Some(Fill::Solid(Color::Scheme("accent2".to_string())))
    );
}

#[test]
fn theme_colors_are_read_from_the_master_theme_part() {
    let deck = read_deck(&deck_bytes()).expect("read");
    let theme = &deck.theme_colors;
    assert_eq!(theme.get("accent1"), Some("4472C4"));
    assert_eq!(theme.get("accent2"), Some("ED7D31"));
    // sysClr resolves through lastClr.
    assert_eq!(theme.get("dk1"), Some("000000"));
    assert_eq!(theme.get("lt1"), Some("FFFFFF"));
    // The bg/tx aliases map onto lt/dk, the way the default clrMap does.
    assert_eq!(theme.get("bg1"), Some("FFFFFF"));
    assert_eq!(theme.get("tx1"), Some("000000"));
    assert_eq!(theme.get("bg2"), Some("E7E6E6"));
    assert_eq!(theme.get("tx2"), Some("44546A"));
}

#[test]
fn the_sp_tree_header_elements_are_not_mistaken_for_shapes() {
    let deck = read_deck(&deck_bytes()).expect("read");
    // sp, sp(noFill), pic, grpSp — the nvGrpSpPr/grpSpPr preamble adds nothing.
    assert_eq!(deck.slides[0].content.shapes.len(), 4);
}

#[test]
fn a_deck_without_a_theme_still_reads() {
    // Drop the theme and master rels: colors fall back to an empty scheme.
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", SLIDE),
            ("ppt/slides/_rels/slide1.xml.rels", SLIDE_RELS),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    let deck = read_deck(&buffer).expect("read");
    assert_eq!(deck.theme_colors.get("accent1"), None);
    assert_eq!(deck.slides[0].content.shapes.len(), 4);
}

/// A one-slide deck around `slide_xml`, without theme or media parts.
fn deck_with_slide(slide_xml: &str) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", slide_xml),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    buffer
}

fn one_shape_slide(sp_inner: &str) -> String {
    format!(
        r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>{sp_inner}</p:sp>
  </p:spTree></p:cSld>
</p:sld>"#
    )
}

#[test]
fn body_properties_carry_the_anchor_and_custom_insets() {
    let slide = one_shape_slide(
        r#"<p:nvSpPr><p:cNvPr id="2" name="Anchored"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm></p:spPr>
        <p:txBody>
          <a:bodyPr anchor="ctr" lIns="182880" tIns="91440"/>
          <a:p><a:r><a:t>centred</a:t></a:r></a:p>
        </p:txBody>"#,
    );
    let deck = read_deck(&deck_with_slide(&slide)).expect("read");
    let ShapeItem::Shape(shape) = &deck.slides[0].content.shapes[0] else {
        panic!("expected shape");
    };
    let text = shape.text.as_ref().expect("text body");
    assert_eq!(text.anchor, Some(jade_pptx_reader::TextAnchor::Center));
    assert_eq!(text.insets.left_emu, Some(182_880));
    assert_eq!(text.insets.top_emu, Some(91_440));
    assert_eq!(text.insets.right_emu, None);
    assert_eq!(text.insets.bottom_emu, None);
}

#[test]
fn a_gradient_fill_keeps_its_stops_in_order() {
    let slide = one_shape_slide(
        r#"<p:nvSpPr><p:cNvPr id="2" name="Gradient"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr>
          <a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
          <a:gradFill>
            <a:gsLst>
              <a:gs pos="100000"><a:schemeClr val="accent1"/></a:gs>
              <a:gs pos="0"><a:srgbClr val="FF0000"/></a:gs>
            </a:gsLst>
            <a:lin ang="5400000" scaled="1"/>
          </a:gradFill>
        </p:spPr>"#,
    );
    let deck = read_deck(&deck_with_slide(&slide)).expect("read");
    let ShapeItem::Shape(shape) = &deck.slides[0].content.shapes[0] else {
        panic!("expected shape");
    };
    let Some(Fill::Gradient { stops, .. }) = &shape.fill else {
        panic!("expected gradient fill, got {:?}", shape.fill);
    };
    // Stops sort by position regardless of document order.
    assert_eq!(stops.len(), 2);
    assert_eq!(stops[0].position_thousandths, 0);
    assert_eq!(stops[0].color, Color::Srgb("FF0000".to_string()));
    assert_eq!(stops[1].position_thousandths, 100_000);
    assert_eq!(stops[1].color, Color::Scheme("accent1".to_string()));
}

#[test]
fn a_slide_background_fill_is_read() {
    let slide = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:bg><p:bgPr><a:solidFill><a:schemeClr val="accent3"/></a:solidFill><a:effectLst/></p:bgPr></p:bg>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr/>
    </p:spTree>
  </p:cSld>
</p:sld>"#;
    let deck = read_deck(&deck_with_slide(slide)).expect("read");
    assert_eq!(
        deck.slides[0].content.background,
        Some(Fill::Solid(Color::Scheme("accent3".to_string())))
    );
}

#[test]
fn a_slide_without_a_background_reads_none() {
    let deck = read_deck(&deck_bytes()).expect("read");
    assert_eq!(deck.slides[0].content.background, None);
}

const SLIDE_LAYOUT: &str = r#"<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Title Placeholder"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="10515600" cy="1325563"/></a:xfrm></p:spPr>
    </p:sp>
  </p:spTree></p:cSld>
</p:sldLayout>"#;

const SLIDE_LAYOUT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>
</Relationships>"#;

const MASTER_WITH_BODY: &str = r#"<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Body Placeholder"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="838200" y="1825625"/><a:ext cx="10515600" cy="4351338"/></a:xfrm></p:spPr>
    </p:sp>
  </p:spTree></p:cSld>
</p:sldMaster>"#;

/// A deck whose slide inherits placeholder geometry: the title placeholder is
/// positioned by the layout, the body placeholder only by the master.
fn inheriting_deck_bytes() -> Vec<u8> {
    let slide = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
      <p:spPr/>
      <p:txBody><a:bodyPr/><a:p><a:r><a:t>Inherited Title</a:t></a:r></a:p></p:txBody>
    </p:sp>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="3" name="Body 2"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
      <p:spPr/>
      <p:txBody><a:bodyPr/><a:p><a:r><a:t>Inherited body</a:t></a:r></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:sld>"#;
    let slide_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
</Relationships>"#;
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", slide),
            ("ppt/slides/_rels/slide1.xml.rels", slide_rels),
            ("ppt/slideLayouts/slideLayout1.xml", SLIDE_LAYOUT),
            (
                "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
                SLIDE_LAYOUT_RELS,
            ),
            ("ppt/slideMasters/slideMaster1.xml", MASTER_WITH_BODY),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    buffer
}

#[test]
fn a_slide_carries_its_layout_and_master_shape_trees() {
    let deck = read_deck(&inheriting_deck_bytes()).expect("read");
    let slide = &deck.slides[0];

    let layout = slide.layout_content.as_ref().expect("layout content");
    let ShapeItem::Shape(layout_title) = &layout.shapes[0] else {
        panic!("expected layout shape");
    };
    assert_eq!(
        layout_title.placeholder.as_ref().map(|ph| ph.kind.as_str()),
        Some("title")
    );
    assert_eq!(
        layout_title.transform.map(|t| t.offset_emu),
        Some((838_200, 365_125))
    );

    let master = slide.master_content.as_ref().expect("master content");
    let ShapeItem::Shape(master_body) = &master.shapes[0] else {
        panic!("expected master shape");
    };
    assert_eq!(
        master_body.placeholder.as_ref().and_then(|ph| ph.index),
        Some(1)
    );
}

#[test]
fn a_deck_without_layout_parts_still_reads() {
    let deck = read_deck(&deck_with_slide(
        r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld></p:sld>"#,
    ))
    .expect("read");
    assert_eq!(deck.slides[0].layout_content, None);
    assert_eq!(deck.slides[0].master_content, None);
}

#[test]
fn bullet_properties_come_back_per_paragraph() {
    let slide = one_shape_slide(
        r#"<p:nvSpPr><p:cNvPr id="2" name="Bullets"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm></p:spPr>
        <p:txBody>
          <a:bodyPr/>
          <a:p><a:pPr lvl="0"><a:buChar char="•"/></a:pPr><a:r><a:t>first</a:t></a:r></a:p>
          <a:p><a:pPr lvl="1"><a:buAutoNum type="arabicPeriod"/></a:pPr><a:r><a:t>second</a:t></a:r></a:p>
          <a:p><a:pPr><a:buNone/></a:pPr><a:r><a:t>third</a:t></a:r></a:p>
          <a:p><a:r><a:t>fourth</a:t></a:r></a:p>
        </p:txBody>"#,
    );
    let deck = read_deck(&deck_with_slide(&slide)).expect("read");
    let ShapeItem::Shape(shape) = &deck.slides[0].content.shapes[0] else {
        panic!("expected shape");
    };
    let paragraphs = &shape.text.as_ref().expect("text").paragraphs;
    assert_eq!(
        paragraphs[0].bullet,
        Some(jade_pptx_reader::Bullet::Char("•".to_string()))
    );
    assert_eq!(
        paragraphs[1].bullet,
        Some(jade_pptx_reader::Bullet::AutoNumber {
            scheme: "arabicPeriod".to_string()
        })
    );
    assert_eq!(paragraphs[2].bullet, Some(jade_pptx_reader::Bullet::None));
    assert_eq!(paragraphs[3].bullet, None);
}
