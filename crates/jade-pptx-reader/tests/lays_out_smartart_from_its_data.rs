//! A SmartArt diagram saved as its data alone — no cached drawing, no
//! layout, colour or style part, which PowerPoint accepts and lays out
//! from its built-in definitions — reads as the shapes PowerPoint would
//! draw for it: one per bullet, inside the frame, in every category.

use std::io::Write;

use jade_pptx_reader::{read_deck, ShapeItem};

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const PRESENTATION: &str = r#"<p:presentation xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/></p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#;

const SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="2" name="Diagram 1"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="2032000" y="719666"/><a:ext cx="8128000" cy="5418667"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds r:dm="rId5" r:lo="" r:qs="" r:cs=""/></a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#;

const SLIDE_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData" Target="../diagrams/data1.xml"/></Relationships>"#;

fn data(layout: &str) -> String {
    let mut model = jade_smartart::DataModel::new(layout, "accent1_2", "simple1");
    model.set_nodes(&[
        jade_smartart::Node::with("Plan", vec![jade_smartart::Node::new("Scope")]),
        jade_smartart::Node::new("Build"),
        jade_smartart::Node::new("Ship"),
    ]);
    model.to_xml()
}

fn shapes(layout: &str) -> Vec<jade_pptx_reader::Shape> {
    let data = data(layout);
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", SLIDE),
            ("ppt/slides/_rels/slide1.xml.rels", SLIDE_RELS),
            ("ppt/diagrams/data1.xml", data.as_str()),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    let deck = read_deck(&buffer).expect("read");
    let ShapeItem::Group(group) = &deck.slides[0].content.shapes[0] else {
        panic!("a group");
    };
    group
        .children
        .iter()
        .filter_map(|item| match item {
            ShapeItem::Shape(shape) => Some(shape.clone()),
            _ => None,
        })
        .collect()
}

fn text(shape: &jade_pptx_reader::Shape) -> String {
    shape
        .text
        .as_ref()
        .map(|body| {
            body.paragraphs
                .iter()
                .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
                .collect::<Vec<_>>()
                .join("|")
        })
        .unwrap_or_default()
}

#[test]
fn every_category_is_laid_out_from_the_data_alone() {
    for layout in [
        "default",
        "process1",
        "cycle2",
        "orgChart1",
        "venn3",
        "matrix3",
        "pyramid1",
        "pList1",
    ] {
        let shapes = shapes(layout);
        let all: Vec<String> = shapes.iter().map(text).collect();
        for word in ["Plan", "Build", "Ship"] {
            assert!(
                all.iter().any(|t| t.split('|').any(|p| p == word)),
                "{layout}: {word} missing from {all:?}"
            );
        }
        for shape in &shapes {
            let t = shape.transform.expect("xfrm");
            assert!(
                t.offset_emu.0 >= -10_000
                    && t.offset_emu.1 >= -10_000
                    && t.offset_emu.0 + t.extent_emu.0 <= 8_138_000
                    && t.offset_emu.1 + t.extent_emu.1 <= 5_428_667,
                "{layout}: {:?} outside the frame",
                t
            );
        }
    }
}

#[test]
fn bullets_take_the_scheme_colours_and_the_style() {
    let shapes = shapes("process1");
    let plan = shapes
        .iter()
        .find(|s| text(s).starts_with("Plan"))
        .expect("Plan");
    assert!(plan.style.is_some(), "a shape style");
    assert!(plan.fill.is_some(), "a fill of its own: {plan:?}");
    assert_eq!(plan.preset_geometry.as_deref(), Some("roundRect"));
}
