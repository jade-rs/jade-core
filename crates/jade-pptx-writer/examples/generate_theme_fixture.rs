//! Synthetic multi-master/override corpus; not a PowerPoint capture reference.
use jade_opc::OpcPackage;
use std::path::Path;
fn replace_block(xml: &str, tag: &str, value: &str) -> String {
    let start = xml.find(&format!("<{tag}")).expect("block start");
    let end = xml[start..].find(&format!("</{tag}>")).expect("block end") + start + tag.len() + 3;
    format!("{}{value}{}", &xml[..start], &xml[end..])
}
fn block<'a>(xml: &'a str, tag: &str) -> &'a str {
    let start = xml.find(&format!("<{tag}")).expect("block start");
    let end = xml[start..].find(&format!("</{tag}>")).expect("block end") + start + tag.len() + 3;
    &xml[start..end]
}
fn rels(items: &[(&str, &str, &str)]) -> String {
    let entries:String=items.iter().map(|(id,kind,target)|format!("<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{target}\"/>")).collect();
    format!("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{entries}</Relationships>")
}
fn theme(base: &str, accent1: &str, accent2: &str, font: &str) -> String {
    base.replace("1E7A5A", accent1)
        .replace("C7913B", accent2)
        .replace("Helvetica Neue", font)
        .replace("Hiragino Kaku Gothic ProN", font)
}
fn lines(xml: &str, width: i64, dash: &str, color: &str) -> String {
    replace_block(xml,"a:lnStyleLst",&format!("<a:lnStyleLst><a:ln w=\"12700\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln><a:ln w=\"{width}\"><a:solidFill>{color}</a:solidFill><a:prstDash val=\"{dash}\"/></a:ln><a:ln w=\"114300\"><a:noFill/><a:prstDash val=\"solid\"/></a:ln></a:lnStyleLst>"))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args().nth(1).expect("output .pptx path");
    let mut p = OpcPackage::open(include_bytes!("../../../public/fixtures/sample-deck.pptx"))?;
    let base = p.required_part_str("ppt/theme/theme1.xml")?;
    let a = lines(
        &theme(&base, "F01414", "14C814", "monospace"),
        76200,
        "sysDash",
        "<a:schemeClr val=\"phClr\"/>",
    );
    let b = lines(
        &theme(&base, "1414F0", "E6A010", "serif"),
        127000,
        "dashDot",
        "<a:schemeClr val=\"phClr\"><a:shade val=\"50000\"/></a:schemeClr>",
    );
    let c = theme(&base, "A020F0", "14C8C8", "sans-serif");
    let d = lines(
        &theme(&base, "F01414", "14C814", "monospace"),
        50800,
        "solid",
        "<a:srgbClr val=\"F014C8\"/>",
    );
    p.replace("ppt/theme/theme1.xml", a.into_bytes());
    p.replace("ppt/theme/theme2.xml", b.into_bytes());
    p.replace("ppt/theme/layoutOverride.xml",format!("<a:themeOverride xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">{}{}</a:themeOverride>",block(&c,"a:clrScheme"),block(&c,"a:fontScheme")).into_bytes());
    let fmt = block(&d, "a:fmtScheme").replace(
        "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>",
        "<a:solidFill><a:srgbClr val=\"F014C8\"/></a:solidFill>",
    );
    p.replace("ppt/theme/slideOverride.xml",format!("<a:themeOverride xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">{}{fmt}</a:themeOverride>",block(&d,"a:fontScheme")).into_bytes());
    let master = p.required_part_str("ppt/slideMasters/slideMaster1.xml")?;
    let a = replace_block(
        &master,
        "p:sldLayoutIdLst",
        "<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst>",
    )
    .replace("tx1=\"dk1\"", "tx1=\"accent2\"");
    let b=replace_block(&master,"p:sldLayoutIdLst","<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483650\" r:id=\"rId1\"/><p:sldLayoutId id=\"2147483651\" r:id=\"rId2\"/></p:sldLayoutIdLst>").replace("tx1=\"dk1\"","tx1=\"accent1\"").replace("accent1=\"accent1\"","accent1=\"accent2\"");
    p.replace("ppt/slideMasters/slideMaster1.xml", a.into_bytes());
    p.replace("ppt/slideMasters/slideMaster2.xml", b.into_bytes());
    p.replace(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        rels(&[
            ("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"),
            ("rId3", "theme", "../theme/theme1.xml"),
        ])
        .into_bytes(),
    );
    p.replace(
        "ppt/slideMasters/_rels/slideMaster2.xml.rels",
        rels(&[
            ("rId1", "slideLayout", "../slideLayouts/slideLayout2.xml"),
            ("rId2", "slideLayout", "../slideLayouts/slideLayout3.xml"),
            ("rId3", "theme", "../theme/theme2.xml"),
        ])
        .into_bytes(),
    );
    let layout = p.required_part_str("ppt/slideLayouts/slideLayout2.xml")?;
    p.replace("ppt/slideLayouts/slideLayout3.xml",layout.replace("<a:masterClrMapping/>","<a:overrideClrMapping bg1=\"lt1\" tx1=\"accent2\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>").into_bytes());
    p.replace(
        "ppt/slideLayouts/_rels/slideLayout2.xml.rels",
        rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster2.xml")]).into_bytes(),
    );
    p.replace(
        "ppt/slideLayouts/_rels/slideLayout3.xml.rels",
        rels(&[
            ("rId1", "slideMaster", "../slideMasters/slideMaster2.xml"),
            ("rId2", "themeOverride", "../theme/layoutOverride.xml"),
        ])
        .into_bytes(),
    );
    for (slide, layout, name, mode) in [
        (1, 1, "Master A", true),
        (2, 2, "Master B", true),
        (3, 3, "Layout override", false),
        (4, 3, "Slide override", true),
    ] {
        let mapping = if mode {
            "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>"
        } else {
            ""
        };
        let shapes = String::from(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Theme text"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="952500" y="952500"/><a:ext cx="5715000" cy="1428750"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:buFont typeface="+mn-lt"/><a:buChar char="•"/></a:pPr><a:r><a:rPr sz="2800"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/></a:rPr><a:t>WWWW iii</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Theme fill"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="952500" y="2857500"/><a:ext cx="1905000" cy="952500"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></p:spPr></p:sp><p:sp><p:nvSpPr><p:cNvPr id="4" name="Theme fill style"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="3810000" y="2857500"/><a:ext cx="1905000" cy="952500"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr><p:style><a:lnRef idx="2"><a:schemeClr val="accent1"/></a:lnRef><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="tx1"/></a:fontRef></p:style></p:sp>"#,
        );
        p.replace(&format!("ppt/slides/slide{slide}.xml"),format!(r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld name="{name}"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{shapes}</p:spTree></p:cSld>{mapping}</p:sld>"#).into_bytes());
        let mut references = rels(&[(
            "rId1",
            "slideLayout",
            &format!("../slideLayouts/slideLayout{layout}.xml"),
        )]);
        if slide == 4 {
            references=references.replace("</Relationships>","<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/themeOverride\" Target=\"../theme/slideOverride.xml\"/></Relationships>");
        }
        p.replace(
            &format!("ppt/slides/_rels/slide{slide}.xml.rels"),
            references.into_bytes(),
        );
    }
    let presentation = p.required_part_str("ppt/presentation.xml")?;
    let presentation = presentation
        .replace(
            "</p:sldMasterIdLst>",
            "<p:sldMasterId id=\"2147483649\" r:id=\"rId7\"/></p:sldMasterIdLst>",
        )
        .replace(
            "</p:sldIdLst>",
            "<p:sldId id=\"259\" r:id=\"rId6\"/></p:sldIdLst>",
        );
    p.replace("ppt/presentation.xml", presentation.into_bytes());
    let references=p.required_part_str("ppt/_rels/presentation.xml.rels")?.replace("</Relationships>","<Relationship Id=\"rId6\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide4.xml\"/><Relationship Id=\"rId7\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster2.xml\"/></Relationships>");
    p.replace("ppt/_rels/presentation.xml.rels", references.into_bytes());
    let mut types = p.required_part_str("[Content_Types].xml")?;
    for (part, kind) in [
        (
            "ppt/slideMasters/slideMaster2.xml",
            "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml",
        ),
        (
            "ppt/slideLayouts/slideLayout3.xml",
            "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml",
        ),
        (
            "ppt/theme/theme2.xml",
            "application/vnd.openxmlformats-officedocument.theme+xml",
        ),
        (
            "ppt/theme/layoutOverride.xml",
            "application/vnd.openxmlformats-officedocument.themeOverride+xml",
        ),
        (
            "ppt/theme/slideOverride.xml",
            "application/vnd.openxmlformats-officedocument.themeOverride+xml",
        ),
        (
            "ppt/slides/slide4.xml",
            "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
        ),
    ] {
        types = types.replace(
            "</Types>",
            &format!("<Override PartName=\"/{part}\" ContentType=\"{kind}\"/></Types>"),
        );
    }
    p.replace("[Content_Types].xml", types.into_bytes());
    let bytes = p.write()?;
    jade_pptx_reader::read_deck(&bytes)?;
    if let Some(parent) = Path::new(&output).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, bytes)?;
    Ok(())
}
