//! A picture's corrections, colour and artistic effects as PowerPoint
//! writes them on the blip and in its `a14:imgProps` extension, the SVG an
//! `asvg:svgBlip` carries, and a freeform it is cropped to.

use jade_pptx_reader::{parse_slide_xml, BlipEffect, Color, ShapeItem};

const SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
<p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>
<p:pic><p:nvPicPr><p:cNvPr id="4" name="図 3"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
<p:blipFill><a:blip r:embed="rId2">
  <a:duotone><a:prstClr val="black"/><a:schemeClr val="accent1"><a:tint val="45000"/></a:schemeClr></a:duotone>
  <a:clrChange><a:clrFrom><a:srgbClr val="FFFFFF"/></a:clrFrom><a:clrTo><a:srgbClr val="FFFFFF"><a:alpha val="0"/></a:srgbClr></a:clrTo></a:clrChange>
  <a:lum bright="20000" contrast="-40000"/>
  <a:extLst>
    <a:ext uri="{BEBA8EAE-BF5A-486C-A8C5-ECC9F3942E4B}"><a14:imgProps xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main"><a14:imgLayer r:embed="rId3"><a14:imgEffect><a14:backgroundRemoval t="5000" b="95000" l="10000" r="90000"/></a14:imgEffect><a14:imgEffect><a14:artisticPencilSketch/></a14:imgEffect><a14:imgEffect><a14:saturation sat="200000"/></a14:imgEffect><a14:imgEffect><a14:colorTemperature colorTemp="8800"/></a14:imgEffect><a14:imgEffect><a14:sharpenSoften amount="-25000"/></a14:imgEffect></a14:imgLayer></a14:imgProps></a:ext>
    <a:ext uri="{96DAC541-7B7A-43D3-8B79-37D633B846F1}"><asvg:svgBlip xmlns:asvg="http://schemas.microsoft.com/office/drawing/2016/SVG/main" r:embed="rId4"/></a:ext>
  </a:extLst>
</a:blip><a:stretch><a:fillRect/></a:stretch></p:blipFill>
<p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></a:xfrm>
<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l="0" t="0" r="r" b="b"/><a:pathLst><a:path w="10" h="10"><a:moveTo><a:pt x="5" y="0"/></a:moveTo><a:lnTo><a:pt x="10" y="10"/></a:lnTo><a:lnTo><a:pt x="0" y="10"/></a:lnTo><a:close/></a:path></a:pathLst></a:custGeom>
</p:spPr></p:pic>
</p:spTree></p:cSld></p:sld>"#;

#[test]
fn a_blips_effects_svg_and_freeform_read_in_order() {
    let content = parse_slide_xml(SLIDE).unwrap();
    let ShapeItem::Picture(picture) = &content.shapes[0] else {
        panic!("a picture");
    };
    assert_eq!(picture.svg_rel_id.as_deref(), Some("rId4"));
    assert!(picture.custom_geometry.is_some());
    let effects = &picture.blip_effects;
    assert!(matches!(
        &effects[0],
        BlipEffect::Duotone { dark: Color::Srgb(black), .. } if black == "000000"
    ));
    assert!(matches!(effects[1], BlipEffect::ColorChange { .. }));
    assert_eq!(
        effects[2],
        BlipEffect::Luminance {
            brightness: 0.2,
            contrast: -0.4
        }
    );
    assert_eq!(
        effects[3],
        BlipEffect::BackgroundRemoval([0.1, 0.05, 0.9, 0.95])
    );
    assert_eq!(
        effects[4],
        BlipEffect::Artistic("artisticPencilSketch".into())
    );
    assert_eq!(effects[5], BlipEffect::Saturation(2.0));
    assert_eq!(effects[6], BlipEffect::Temperature(8800.0));
    assert_eq!(effects[7], BlipEffect::Sharpen(-0.25));
}
