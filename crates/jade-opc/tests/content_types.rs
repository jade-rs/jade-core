//! A part written into a package that declares every part by `Override` —
//! no `Default` for `.rels` or pictures — still has a content type, or
//! Office repairs the file.
use jade_opc::OpcPackage;
use std::io::{Cursor, Write};

const TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/_rels/.rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/></Types>"#;

fn package() -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, text) in [
        ("[Content_Types].xml", TYPES),
        ("_rels/.rels", "<Relationships/>"),
        ("ppt/slides/slide1.xml", "<sld/>"),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn types_of(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .part_str("[Content_Types].xml")
        .unwrap()
        .unwrap()
}

#[test]
fn a_new_relationships_part_and_picture_get_defaults() {
    let original = package();
    for preserving in [false, true] {
        let written = if preserving {
            let mut package = OpcPackage::open_preserving(&original).unwrap();
            package.replace(
                "ppt/slides/_rels/slide1.xml.rels",
                b"<Relationships/>".to_vec(),
            );
            package.replace("ppt/media/image1.PNG", vec![0; 4]);
            package.write().unwrap()
        } else {
            let mut package = OpcPackage::open(&original).unwrap();
            package.replace(
                "ppt/slides/_rels/slide1.xml.rels",
                b"<Relationships/>".to_vec(),
            );
            package.replace("ppt/media/image1.PNG", vec![0; 4]);
            package.write().unwrap()
        };
        let types = types_of(&written);
        assert!(
            types.contains(r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#),
            "{types}"
        );
        assert!(
            types.contains(r#"<Default Extension="png" ContentType="image/png"/>"#),
            "{types}"
        );
        assert!(types.ends_with("</Types>"));
    }
}

#[test]
fn a_package_whose_parts_all_have_types_keeps_its_content_types() {
    let original = package();
    let mut package = OpcPackage::open(&original).unwrap();
    package.replace("ppt/slides/slide1.xml", b"<sld>changed</sld>".to_vec());
    assert_eq!(types_of(&package.write().unwrap()), TYPES);
}
