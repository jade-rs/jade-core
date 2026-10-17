//! Synthetic inheritance corpus; not a PowerPoint-authored visual reference.
use jade_opc::OpcPackage;
use std::{error::Error, fs};
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
fn tree(shapes: &str) -> String {
    format!("<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{shapes}</p:spTree>")
}
fn shape(id: u32, pr: &str, body: &str, text: &str) -> String {
    format!("<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Inherited body\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"body\" idx=\"1\"/></p:nvPr></p:nvSpPr><p:spPr>{pr}</p:spPr><p:txBody>{body}<a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>")
}
fn frame(x: i64, y: i64, w: i64, h: i64) -> String {
    format!(
        "<a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        x * 9525,
        y * 9525,
        w * 9525,
        h * 9525
    )
}
fn replace_element(xml: &str, name: &str, value: &str) -> String {
    let start = xml.find(&format!("<{name}")).unwrap();
    let end = xml[start..].find(&format!("</{name}>")).unwrap() + start + name.len() + 3;
    format!("{}{}{}", &xml[..start], value, &xml[end..])
}
fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args().nth(1).ok_or("output path required")?;
    let mut p = OpcPackage::open(include_bytes!("../../../public/fixtures/sample-deck.pptx"))?;
    let body="<a:bodyPr anchor=\"ctr\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" wrap=\"none\" vert=\"vert270\"><a:normAutofit fontScale=\"50000\"/></a:bodyPr><a:lstStyle><a:lvl1pPr><a:defRPr sz=\"3000\"><a:latin typeface=\"serif\"/></a:defRPr></a:lvl1pPr></a:lstStyle>";
    let pr=format!("{}<a:prstGeom prst=\"roundRect\"><a:avLst><a:gd name=\"adj\" fmla=\"val 25000\"/></a:avLst></a:prstGeom><a:solidFill><a:srgbClr val=\"E6A010\"/></a:solidFill><a:ln w=\"50800\"><a:solidFill><a:srgbClr val=\"1414F0\"/></a:solidFill><a:prstDash val=\"dash\"/></a:ln>",frame(100,100,400,180));
    let master_shape=shape(99,&pr,body,"").replace("idx=\"1\"", "idx=\"41\"").replace("</p:spPr>","</p:spPr><p:style><a:lnRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:lnRef><a:fillRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:fillRef><a:effectRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:effectRef><a:fontRef idx=\"minor\"><a:srgbClr val=\"A020F0\"/></a:fontRef></p:style>");
    let decoy_master = shape(
        100,
        &format!(
            "{}<a:solidFill><a:srgbClr val=\"FF0000\"/></a:solidFill>",
            frame(20, 20, 100, 100)
        ),
        "<a:bodyPr/><a:lstStyle/>",
        "",
    )
    .replace("type=\"body\"", "type=\"title\"");
    let master_shape = format!("{master_shape}{decoy_master}");
    let master = "ppt/slideMasters/slideMaster1.xml";
    let xml = p.required_part_str(master)?;
    let xml = replace_element(&xml, "p:spTree", &tree(&master_shape));
    let xml = replace_element(
        &xml,
        "p:txStyles",
        "<p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles>",
    );
    p.replace(master, xml.into_bytes());
    for index in 1..=2 {
        let mut shapes = shape(20, "", "<a:bodyPr/><a:lstStyle/>", "");
        if index == 2 {
            shapes=shape(20,"<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val=\"14C8C8\"/></a:solidFill>","<a:bodyPr wrap=\"square\" vert=\"horz\"><a:noAutofit/></a:bodyPr><a:lstStyle/>","");
            shapes.push_str(&format!("<p:pic><p:nvPicPr><p:cNvPr id=\"21\" name=\"Picture frame\"/><p:cNvPicPr/><p:nvPr><p:ph type=\"pic\" idx=\"2\"/></p:nvPr></p:nvPicPr><p:blipFill><a:blip r:embed=\"rIdImage\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",frame(650,100,300,180)));
            for (id, idx, x, color) in [(22, 9, 650, "A020F0"), (23, 0, 100, "1414F0")] {
                let title = shape(
                    id,
                    &format!(
                        "{}<a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill>",
                        frame(x, 400, 400, 100)
                    ),
                    "<a:bodyPr/><a:lstStyle/>",
                    "",
                )
                .replace(
                    "type=\"body\" idx=\"1\"",
                    &format!("type=\"title\" idx=\"{idx}\""),
                );
                shapes.push_str(&title);
            }
        }
        p.replace(&format!("ppt/slideLayouts/slideLayout{index}.xml"),format!("<p:sldLayout xmlns:p=\"{P}\" xmlns:a=\"{A}\" xmlns:r=\"{R}\" type=\"obj\"><p:cSld name=\"Inheritance {index}\">{}</p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>",tree(&shapes)).into_bytes());
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        32,
        16,
        image::Rgb([240, 20, 20]),
    ))
    .write_to(&mut png, image::ImageFormat::Png)?;
    p.replace("ppt/media/inherited.png", png.into_inner());
    let ctypes = p.required_part_str("[Content_Types].xml")?;
    if !ctypes.contains("Extension=\"png\"") {
        p.replace(
            "[Content_Types].xml",
            ctypes
                .replace(
                    "</Types>",
                    "<Default Extension=\"png\" ContentType=\"image/png\"/></Types>",
                )
                .into_bytes(),
        );
    }
    let layoutrels = "ppt/slideLayouts/_rels/slideLayout2.xml.rels";
    let rels = p.required_part_str(layoutrels)?;
    p.replace(layoutrels,rels.replace("</Relationships>",&format!("<Relationship Id=\"rIdImage\" Type=\"{R}/image\" Target=\"../media/inherited.png\"/></Relationships>")).into_bytes());
    let text = "Inherited words that keep their master formatting and geometry";
    for index in 1..=3 {
        let mut shapes = if index == 3 {
            "<p:pic><p:nvPicPr><p:cNvPr id=\"2\" name=\"Inherited picture\"/><p:cNvPicPr/><p:nvPr><p:ph type=\"pic\" idx=\"2\"/></p:nvPr></p:nvPicPr><p:blipFill><a:blip r:embed=\"rIdImage\"/><a:srcRect l=\"10000\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr/></p:pic>".into()
        } else if index == 2 {
            shape(2,"<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln>","<a:bodyPr wrap=\"square\" vert=\"horz\"><a:noAutofit/></a:bodyPr><a:lstStyle/>",text)
        } else {
            shape(2, "", "<a:bodyPr/><a:lstStyle/>", text)
        };
        if index == 3 {
            shapes.push_str(
                &shape(3, "", "<a:bodyPr/><a:lstStyle/>", "Default zero title")
                    .replace("type=\"body\" idx=\"1\"", "type=\"title\""),
            );
        }
        p.replace(&format!("ppt/slides/slide{index}.xml"),format!("<p:sld xmlns:p=\"{P}\" xmlns:a=\"{A}\" xmlns:r=\"{R}\"><p:cSld name=\"Inherited {index}\">{}</p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>",tree(&shapes)).into_bytes());
        p.replace(&format!("ppt/slides/_rels/slide{index}.xml.rels"),format!("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{R}/slideLayout\" Target=\"../slideLayouts/slideLayout{}.xml\"/>{}</Relationships>",if index==3 {2}else{1},if index==3 {format!("<Relationship Id=\"rIdImage\" Type=\"{R}/image\" Target=\"../media/inherited.png\"/>")}else{String::new()}).into_bytes());
    }
    fs::write(path, p.write()?)?;
    Ok(())
}
