use std::io::Write;

use jade_pptx_reader::{read_pptx, SlideSize};

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const PRESENTATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>
  <p:sldIdLst>
    <p:sldId id="257" r:id="rId3"/>
    <p:sldId id="256" r:id="rId2"/>
  </p:sldIdLst>
  <p:sldSz cx="9144000" cy="6858000"/>
  <p:notesSz cx="6858000" cy="9144000"/>
</p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide2.xml"/>
</Relationships>"#;

const SLIDE: &str = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree/></p:cSld></p:sld>"#;

fn deck_bytes(presentation: &str) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", presentation),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", SLIDE),
            ("ppt/slides/slide2.xml", SLIDE),
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
fn slides_come_back_in_presentation_order_not_part_order() {
    // `sldIdLst` lists slide2 first; the deck plays in that order even though
    // the parts are numbered the other way.
    let presentation = read_pptx(&deck_bytes(PRESENTATION)).expect("read");
    let parts: Vec<&str> = presentation
        .slides
        .iter()
        .map(|slide| slide.part_name.as_str())
        .collect();
    assert_eq!(parts, ["ppt/slides/slide2.xml", "ppt/slides/slide1.xml"]);
    assert_eq!(presentation.slides[0].id, Some(257));
}

#[test]
fn the_slide_master_relationship_is_not_mistaken_for_a_slide() {
    let presentation = read_pptx(&deck_bytes(PRESENTATION)).expect("read");
    assert_eq!(presentation.slides.len(), 2);
}

#[test]
fn slide_size_is_read_and_converts_to_pixels() {
    let presentation = read_pptx(&deck_bytes(PRESENTATION)).expect("read");
    assert_eq!(
        presentation.slide_size,
        SlideSize {
            width_emu: 9_144_000,
            height_emu: 6_858_000
        }
    );
    // 10 × 7.5 inches at 96 dpi.
    assert_eq!(presentation.slide_size.width_px(), 960.0);
    assert_eq!(presentation.slide_size.height_px(), 720.0);
}

#[test]
fn a_presentation_without_a_size_falls_back_to_the_16_by_9_default() {
    let without_size = PRESENTATION.replace(r#"<p:sldSz cx="9144000" cy="6858000"/>"#, "");
    let presentation = read_pptx(&deck_bytes(&without_size)).expect("read");
    assert_eq!(presentation.slide_size, SlideSize::default());
}

#[test]
fn an_unreadable_slide_list_still_finds_a_slide_part() {
    let without_list = PRESENTATION
        .replace("257", "not-a-number")
        .replace(r#"<p:sldId id="256" r:id="rId2"/>"#, "");
    let presentation = read_pptx(&deck_bytes(&without_list)).expect("read");
    assert_eq!(presentation.slides.len(), 1);
}
