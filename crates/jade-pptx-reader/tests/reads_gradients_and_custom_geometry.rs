//! Gradient fills with their direction, the theme's fill styles that a
//! `fillRef` points at, and custom geometry paths.

use std::io::Write;

use jade_pptx_reader::{read_deck, Color, Deck, Fill, PathCommand, ShapeItem};

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const PRESENTATION: &str = r#"<p:presentation xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/></p:presentation>"#;

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
      <a:dk1><a:srgbClr val="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
    </a:clrScheme>
    <a:fmtScheme name="Office">
      <a:fillStyleLst>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:gradFill rotWithShape="1">
          <a:gsLst>
            <a:gs pos="0"><a:schemeClr val="phClr"><a:lumMod val="110000"/><a:satMod val="105000"/><a:tint val="67000"/></a:schemeClr></a:gs>
            <a:gs pos="100000"><a:schemeClr val="phClr"><a:lumMod val="105000"/><a:tint val="81000"/></a:schemeClr></a:gs>
          </a:gsLst>
          <a:lin ang="5400000" scaled="0"/>
        </a:gradFill>
        <a:gradFill rotWithShape="1">
          <a:gsLst>
            <a:gs pos="0"><a:schemeClr val="phClr"><a:shade val="63000"/></a:schemeClr></a:gs>
            <a:gs pos="100000"><a:schemeClr val="phClr"/></a:gs>
          </a:gsLst>
          <a:lin ang="5400000" scaled="0"/>
        </a:gradFill>
      </a:fillStyleLst>
      <a:bgFillStyleLst>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:solidFill><a:schemeClr val="phClr"><a:tint val="95000"/></a:schemeClr></a:solidFill>
        <a:gradFill><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"/></a:gs><a:gs pos="100000"><a:schemeClr val="phClr"><a:shade val="50000"/></a:schemeClr></a:gs></a:gsLst><a:path path="circle"><a:fillToRect l="50000" t="50000" r="50000" b="50000"/></a:path></a:gradFill>
      </a:bgFillStyleLst>
    </a:fmtScheme>
  </a:themeElements>
</a:theme>"#;

const SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:bg><p:bgPr><a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="FFFFFF"/></a:gs><a:gs pos="100000"><a:srgbClr val="000000"/></a:gs></a:gsLst><a:lin ang="2700000" scaled="1"/></a:gradFill></p:bgPr></p:bg>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr/>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Angled"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr>
          <a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
          <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
          <a:gradFill><a:gsLst><a:gs pos="100000"><a:srgbClr val="0000FF"/></a:gs><a:gs pos="0"><a:srgbClr val="FF0000"/></a:gs></a:gsLst><a:lin ang="5400000" scaled="0"/></a:gradFill>
        </p:spPr>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Unangled"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr>
          <a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
          <a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="FF0000"/></a:gs><a:gs pos="100000"><a:srgbClr val="0000FF"/></a:gs></a:gsLst></a:gradFill>
        </p:spPr>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="4" name="Freeform"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr>
          <a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
          <a:custGeom>
            <a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/>
            <a:rect l="0" t="0" r="r" b="b"/>
            <a:pathLst>
              <a:path w="200" h="100">
                <a:moveTo><a:pt x="0" y="0"/></a:moveTo>
                <a:lnTo><a:pt x="200" y="0"/></a:lnTo>
                <a:cubicBezTo><a:pt x="200" y="50"/><a:pt x="150" y="100"/><a:pt x="100" y="100"/></a:cubicBezTo>
                <a:quadBezTo><a:pt x="50" y="100"/><a:pt x="0" y="50"/></a:quadBezTo>
                <a:arcTo wR="10" hR="10" stAng="0" swAng="5400000"/>
                <a:close/>
              </a:path>
              <a:path w="200" h="100" fill="none" stroke="0">
                <a:moveTo><a:pt x="10" y="10"/></a:moveTo>
                <a:lnTo><a:pt x="20" y="10"/></a:lnTo>
              </a:path>
            </a:pathLst>
          </a:custGeom>
          <a:solidFill><a:srgbClr val="00FF00"/></a:solidFill>
        </p:spPr>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"#;

fn deck() -> Deck {
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
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    read_deck(&buffer).expect("read")
}

fn shape(deck: &Deck, index: usize) -> &jade_pptx_reader::Shape {
    match &deck.slides[0].content.shapes[index] {
        ShapeItem::Shape(shape) => shape,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_linear_gradient_carries_its_angle_and_sorted_stops() {
    let deck = deck();
    let Some(Fill::Gradient {
        stops, angle_deg, ..
    }) = &shape(&deck, 0).fill
    else {
        panic!("gradient");
    };
    assert_eq!(*angle_deg, Some(90.0));
    let positions: Vec<u32> = stops.iter().map(|stop| stop.position_thousandths).collect();
    assert_eq!(positions, vec![0, 100_000]);
    assert_eq!(stops[0].color, Color::Srgb("FF0000".into()));

    let Some(Fill::Gradient { angle_deg, .. }) = &shape(&deck, 1).fill else {
        panic!("gradient");
    };
    assert_eq!(*angle_deg, None);
}

#[test]
fn a_slide_background_gradient_keeps_its_angle() {
    let deck = deck();
    let Some(Fill::Gradient {
        angle_deg, stops, ..
    }) = &deck.slides[0].content.background
    else {
        panic!("background gradient");
    };
    assert_eq!(*angle_deg, Some(45.0));
    assert_eq!(stops.len(), 2);
}

#[test]
fn the_theme_fill_styles_are_read_with_their_placeholder_colour() {
    let deck = deck();
    assert_eq!(deck.theme_fill_styles.len(), 3);
    assert_eq!(
        deck.theme_fill_styles[0],
        Fill::Solid(Color::Scheme("phClr".into()))
    );
    let Fill::Gradient {
        stops, angle_deg, ..
    } = &deck.theme_fill_styles[1]
    else {
        panic!("second fill style is a gradient");
    };
    assert_eq!(*angle_deg, Some(90.0));
    assert_eq!(
        stops[0].color,
        // satMod is beyond `Adjusted`, so every transform is kept in order.
        Color::Transformed {
            base: Box::new(Color::Scheme("phClr".into())),
            ops: vec![
                ("lumMod".into(), 1.1),
                ("satMod".into(), 1.05),
                ("tint".into(), 0.67),
            ],
        }
    );
    assert_eq!(deck.theme_background_fill_styles.len(), 3);
    // A path gradient has no angle.
    let Fill::Gradient { angle_deg, .. } = &deck.theme_background_fill_styles[2] else {
        panic!("third background style is a gradient");
    };
    assert_eq!(*angle_deg, None);
}

#[test]
fn custom_geometry_reads_its_paths_and_commands() {
    let deck = deck();
    let freeform = shape(&deck, 2);
    assert_eq!(freeform.preset_geometry, None);
    let geometry = freeform.custom_geometry.as_ref().expect("custGeom");
    assert_eq!(geometry.paths.len(), 2);
    let path = &geometry.paths[0];
    assert_eq!((path.width, path.height), (200, 100));
    assert!(path.filled);
    assert!(path.stroked);
    assert_eq!(
        path.commands,
        vec![
            PathCommand::MoveTo(0, 0),
            PathCommand::LineTo(200, 0),
            PathCommand::CubicTo {
                control1: (200, 50),
                control2: (150, 100),
                to: (100, 100),
            },
            PathCommand::QuadTo {
                control: (50, 100),
                to: (0, 50),
            },
            PathCommand::ArcTo {
                width_radius: 10,
                height_radius: 10,
                start_60000ths: 0,
                sweep_60000ths: 5_400_000,
            },
            PathCommand::Close,
        ]
    );
    let outline_only = &geometry.paths[1];
    assert!(!outline_only.filled);
    assert!(!outline_only.stroked);
    assert_eq!(outline_only.commands.len(), 2);
}
