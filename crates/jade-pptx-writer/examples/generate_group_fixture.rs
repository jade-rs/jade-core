use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use jade_opc::OpcPackage;
use jade_pptx_reader::Transform;
use jade_pptx_writer::{EditSession, TextFormat};
use std::{io::Cursor, path::Path};
fn transform(x: i64, y: i64, w: i64, h: i64) -> Transform {
    Transform {
        offset_emu: (x * 9525, y * 9525),
        extent_emu: (w * 9525, h * 9525),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    }
}
fn picture(color: [u8; 3]) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 32, Rgb(color)))
        .write_to(&mut out, ImageFormat::Png)
        .unwrap();
    out.into_inner()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args().nth(1).expect("output .pptx path");
    let mut s = EditSession::open(include_bytes!("../../../public/fixtures/sample-deck.pptx"))?;
    while jade_pptx_reader::read_deck(s.bytes())?.slides.len() > 1 {
        s.delete_slide(1)?;
    }
    for shape in jade_pptx_writer::editable_shapes(s.bytes(), 0)? {
        if shape.can_delete {
            s.delete_shape(0, shape.id)?;
        }
    }
    s.insert_picture(0, transform(0, 0, 120, 80), &picture([240, 20, 20]))?;
    s.insert_picture(0, transform(200, 100, 120, 80), &picture([20, 20, 240]))?;
    s.insert_text_box(
        0,
        transform(30, 90, 140, 80),
        "Group text",
        &TextFormat::default(),
    )?;
    let mut package = OpcPackage::open(s.bytes())?;
    let xml = package.required_part_str("ppt/slides/slide1.xml")?;
    let start = xml.find("<jadeSp:pic ").unwrap();
    let end = xml.rfind("</jadeSp:sp>").unwrap() + "</jadeSp:sp>".len();
    let children = &xml[start..end];
    let red_end = children.find("</jadeSp:pic>").unwrap() + "</jadeSp:pic>".len();
    let group = |id: u32,
                 name: &str,
                 frame: Transform,
                 child: Transform,
                 rot: i32,
                 flip: bool,
                 children: &str| {
        format!("<g:grpSp xmlns:g=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:d=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><g:nvGrpSpPr><g:cNvPr id=\"{id}\" name=\"{name}\"/><g:cNvGrpSpPr/><g:nvPr/></g:nvGrpSpPr><g:grpSpPr><d:xfrm rot=\"{rot}\" flipV=\"{}\"><d:off x=\"{}\" y=\"{}\"/><d:ext cx=\"{}\" cy=\"{}\"/><d:chOff x=\"{}\" y=\"{}\"/><d:chExt cx=\"{}\" cy=\"{}\"/></d:xfrm></g:grpSpPr>{children}</g:grpSp>",if flip{1}else{0},frame.offset_emu.0,frame.offset_emu.1,frame.extent_emu.0,frame.extent_emu.1,child.offset_emu.0,child.offset_emu.1,child.extent_emu.0,child.extent_emu.1)
    };
    let inner = group(
        51,
        "Inner group",
        transform(20, 20, 120, 80),
        transform(0, 0, 120, 80),
        1800000,
        true,
        &children[..red_end],
    );
    let namespaces = "xmlns:g=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:d=\"http://schemas.openxmlformats.org/drawingml/2006/main\"";
    let connector = |id: u32, name: &str, start: u32, end: u32, t: Transform| {
        format!("<g:cxnSp {namespaces}><g:nvCxnSpPr><g:cNvPr id=\"{id}\" name=\"{name}\"/><g:cNvCxnSpPr><d:stCxn id=\"{start}\" idx=\"0\"/><d:endCxn id=\"{end}\" idx=\"0\"/></g:cNvCxnSpPr><g:nvPr/></g:nvCxnSpPr><g:spPr><d:xfrm><d:off x=\"{}\" y=\"{}\"/><d:ext cx=\"{}\" cy=\"{}\"/></d:xfrm><d:prstGeom prst=\"line\"><d:avLst/></d:prstGeom><d:ln w=\"9525\"><d:solidFill><d:srgbClr val=\"000000\"/></d:solidFill></d:ln></g:spPr></g:cxnSp>",t.offset_emu.0,t.offset_emu.1,t.extent_emu.0,t.extent_emu.1)
    };
    let inside = connector(60, "Inside connector", 2, 99, transform(140, 80, 50, 10));
    let outer = group(
        50,
        "Outer group",
        transform(300, 200, 400, 200),
        transform(0, 0, 400, 200),
        0,
        false,
        &format!("{inner}{}{inside}", &children[red_end..]),
    ).replace("<g:cNvPr id=\"50\" name=\"Outer group\"/>","<g:cNvPr id=\"50\" name=\"Outer group\"><d:extLst><d:ext uri=\"{FF2B5EF4-FFF2-40B4-BE49-F238E27FC236}\"><a16:creationId xmlns:a16=\"http://schemas.microsoft.com/office/drawing/2014/main\" id=\"{0980FF19-E7E7-493C-8D3E-15B2100EA940}\"/></d:ext></d:extLst></g:cNvPr>");
    let outside = connector(70, "Outside connector", 3, 99, transform(50, 500, 200, 30));
    let other=format!("<g:sp {namespaces}><g:nvSpPr><g:cNvPr id=\"99\" name=\"Outside target\"/><g:cNvSpPr/><g:nvPr/></g:nvSpPr><g:spPr><d:xfrm><d:off x=\"285750\" y=\"4953000\"/><d:ext cx=\"571500\" cy=\"381000\"/></d:xfrm><d:prstGeom prst=\"rect\"><d:avLst/></d:prstGeom><d:solidFill><d:srgbClr val=\"20B020\"/></d:solidFill></g:spPr></g:sp>");
    package.replace(
        "ppt/slides/slide1.xml",
        format!("{}{outer}{outside}{other}{}", &xml[..start], &xml[end..]).replace("</p:sld>","<p:timing><p:bldLst><p:bldP spid=\"4\" grpId=\"0\"/></p:bldLst></p:timing></p:sld>").into_bytes(),
    );
    let bytes = package.write()?;
    jade_pptx_reader::read_deck(&bytes)?;
    std::fs::write(Path::new(&output), bytes)?;
    Ok(())
}
