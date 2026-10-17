//! The fourth reader slice: preset adjustment values, picture placeholders,
//! run capitalisation, numbering start values, and per-slide colour map
//! overrides.

use std::io::Write;

use jade_pptx_reader::{read_deck, Bullet, Deck, ShapeItem};

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const PRESENTATION: &str = r#"<p:presentation xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/></p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#;

const SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Rounded"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
      <p:spPr>
        <a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
        <a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="val 5000"/></a:avLst></a:prstGeom>
      </p:spPr>
      <p:txBody><a:bodyPr/>
        <a:p><a:pPr><a:buAutoNum type="alphaLcParenR" startAt="3"/></a:pPr><a:r><a:rPr cap="all"/><a:t>shout</a:t></a:r></a:p>
        <a:p><a:r><a:rPr cap="none"/><a:t>quiet</a:t></a:r></a:p>
      </p:txBody>
    </p:sp>
    <p:pic>
      <p:nvPicPr><p:cNvPr id="3" name="Picture Placeholder 2"/><p:cNvPicPr/><p:nvPr><p:ph type="pic" idx="13"/></p:nvPr></p:nvPicPr>
      <p:blipFill><a:blip r:embed="rId9"/></p:blipFill>
      <p:spPr/>
    </p:pic>
  </p:spTree></p:cSld>
  <p:clrMapOvr><a:overrideClrMapping bg1="dk1" tx1="lt1" bg2="dk2" tx2="lt2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/></p:clrMapOvr>
</p:sld>"#;

fn deck() -> Deck {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
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

#[test]
fn preset_adjustment_values_are_read() {
    let deck = deck();
    let ShapeItem::Shape(shape) = &deck.slides[0].content.shapes[0] else {
        panic!("shape first");
    };
    assert_eq!(shape.adjustments, vec![("adj".to_string(), 5000)]);
}

#[test]
fn a_picture_can_be_a_placeholder() {
    let deck = deck();
    let ShapeItem::Picture(picture) = &deck.slides[0].content.shapes[1] else {
        panic!("picture second");
    };
    let placeholder = picture.placeholder.as_ref().expect("p:ph on the picture");
    assert_eq!(placeholder.kind, "pic");
    assert_eq!(placeholder.index, Some(13));
    assert!(picture.transform.is_none());
}

#[test]
fn run_capitalisation_and_numbering_start_are_read() {
    let deck = deck();
    let ShapeItem::Shape(shape) = &deck.slides[0].content.shapes[0] else {
        panic!("shape first");
    };
    let text = shape.text.as_ref().expect("text");
    assert!(text.paragraphs[0].runs[0].all_caps);
    assert!(!text.paragraphs[1].runs[0].all_caps);
    assert_eq!(
        text.paragraphs[0].bullet,
        Some(Bullet::AutoNumber {
            scheme: "alphaLcParenR".to_string()
        })
    );
    assert_eq!(text.paragraphs[0].number_start, Some(3));
    assert_eq!(text.paragraphs[1].number_start, None);
}

#[test]
fn a_slide_colour_map_override_is_read() {
    let deck = deck();
    let map = deck.slides[0]
        .content
        .color_map_override
        .as_ref()
        .expect("p:clrMapOvr");
    assert_eq!(map.get("bg1").map(String::as_str), Some("dk1"));
    assert_eq!(map.get("tx1").map(String::as_str), Some("lt1"));
}
