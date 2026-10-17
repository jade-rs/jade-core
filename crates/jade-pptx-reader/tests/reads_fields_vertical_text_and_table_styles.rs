//! Slide number fields resolved to the slide's real number, vertical text
//! bodies, hidden slides, and table style references — the built-in GUID a
//! PowerPoint table names and the custom styles a deck may carry.

use std::io::Write;

use jade_pptx_reader::{read_deck, Color, Deck, Fill, ShapeItem, TextVertical};

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const PRESENTATION: &str = r#"<p:presentation xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/></p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide2.xml"/>
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/tableStyles" Target="tableStyles.xml"/>
</Relationships>"#;

const TABLE_STYLES: &str = r#"<a:tblStyleLst xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" def="{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}">
  <a:tblStyle styleId="{CUSTOM-1}" styleName="Custom">
    <a:wholeTbl>
      <a:tcTxStyle b="on"><a:fontRef idx="minor"><a:prstClr val="black"/></a:fontRef><a:schemeClr val="dk1"/></a:tcTxStyle>
      <a:tcStyle>
        <a:tcBdr>
          <a:left><a:ln w="12700"><a:solidFill><a:schemeClr val="lt1"/></a:solidFill></a:ln></a:left>
          <a:insideH><a:ln w="6350"><a:solidFill><a:srgbClr val="ABCDEF"/></a:solidFill></a:ln></a:insideH>
          <a:bottom><a:lnRef idx="1"><a:schemeClr val="accent1"/></a:lnRef></a:bottom>
        </a:tcBdr>
        <a:fill><a:solidFill><a:schemeClr val="accent1"><a:tint val="20000"/></a:schemeClr></a:solidFill></a:fill>
      </a:tcStyle>
    </a:wholeTbl>
    <a:firstRow>
      <a:tcTxStyle b="on"><a:schemeClr val="lt1"/></a:tcTxStyle>
      <a:tcStyle><a:tcBdr/><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef></a:tcStyle>
    </a:firstRow>
  </a:tblStyle>
</a:tblStyleLst>"#;

const SLIDE_ONE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" show="0">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Vertical"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm></p:spPr>
      <p:txBody><a:bodyPr vert="eaVert"/><a:p><a:r><a:t>縦書き</a:t></a:r></a:p></p:txBody>
    </p:sp>
    <p:graphicFrame>
      <p:nvGraphicFramePr><p:cNvPr id="3" name="Table 2"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
      <p:xfrm><a:off x="0" y="0"/><a:ext cx="1828800" cy="384048"/></p:xfrm>
      <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
        <a:tbl>
          <a:tblPr firstRow="1" bandRow="1"><a:tableStyleId>{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}</a:tableStyleId></a:tblPr>
          <a:tblGrid><a:gridCol w="914400"/><a:gridCol w="914400"/></a:tblGrid>
          <a:tr h="384048"><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>A</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>B</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr>
        </a:tbl>
      </a:graphicData></a:graphic>
    </p:graphicFrame>
  </p:spTree></p:cSld>
</p:sld>"#;

const SLIDE_TWO: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Slide Number 1"/><p:cNvSpPr/><p:nvPr><p:ph type="sldNum" idx="12"/></p:nvPr></p:nvSpPr>
      <p:spPr/>
      <p:txBody><a:bodyPr/><a:p><a:fld id="{F1}" type="slidenum"><a:rPr lang="en-US"/><a:t>7</a:t></a:fld><a:r><a:t> of many</a:t></a:r></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:sld>"#;

fn deck() -> Deck {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/tableStyles.xml", TABLE_STYLES),
            ("ppt/slides/slide1.xml", SLIDE_ONE),
            ("ppt/slides/slide2.xml", SLIDE_TWO),
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
fn a_slide_number_field_shows_the_slides_real_number() {
    let deck = deck();
    let ShapeItem::Shape(shape) = &deck.slides[1].content.shapes[0] else {
        panic!("shape");
    };
    let runs = &shape.text.as_ref().expect("text").paragraphs[0].runs;
    // The cached "7" is stale: this is slide 2.
    assert_eq!(runs[0].text, "2");
    assert_eq!(runs[0].field.as_deref(), Some("slidenum"));
    assert_eq!(runs[1].text, " of many");
    assert_eq!(runs[1].field, None);
}

#[test]
fn a_hidden_slide_is_flagged() {
    let deck = deck();
    assert!(deck.slides[0].content.hidden);
    assert!(!deck.slides[1].content.hidden);
}

#[test]
fn a_vertical_text_body_records_its_direction() {
    let deck = deck();
    let ShapeItem::Shape(shape) = &deck.slides[0].content.shapes[0] else {
        panic!("shape");
    };
    assert_eq!(
        shape.text.as_ref().expect("text").vertical,
        Some(TextVertical::EastAsian)
    );
    let ShapeItem::Shape(shape) = &deck.slides[1].content.shapes[0] else {
        panic!("shape");
    };
    assert_eq!(shape.text.as_ref().expect("text").vertical, None);
}

#[test]
fn a_table_records_its_style_id_and_option_flags() {
    let deck = deck();
    let ShapeItem::Table(table) = &deck.slides[0].content.shapes[1] else {
        panic!("table");
    };
    assert_eq!(
        table.style.style_id.as_deref(),
        Some("{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}")
    );
    assert!(table.style.first_row);
    assert!(table.style.band_rows);
    assert!(!table.style.last_row);
    assert!(!table.style.first_col);
}

#[test]
fn custom_table_styles_are_read_from_the_table_styles_part() {
    let deck = deck();
    assert_eq!(deck.table_styles.len(), 1);
    let style = &deck.table_styles[0];
    assert_eq!(style.id, "{CUSTOM-1}");

    let whole = style.whole.as_ref().expect("wholeTbl");
    assert_eq!(whole.bold, Some(true));
    assert_eq!(whole.text_color, Some(Color::Scheme("dk1".into())));
    assert_eq!(
        whole.fill,
        Some(Fill::Solid(Color::Adjusted {
            base: Box::new(Color::Scheme("accent1".into())),
            alpha: None,
            lum_mod: None,
            lum_off: None,
            shade: None,
            tint: Some(0.2),
        }))
    );
    let left = whole.left.as_ref().expect("left border");
    assert_eq!(left.width_emu, Some(12_700));
    assert_eq!(left.color, Some(Color::Scheme("lt1".into())));
    let inside_h = whole.inside_h.as_ref().expect("insideH");
    assert_eq!(inside_h.color, Some(Color::Srgb("ABCDEF".into())));
    // A lnRef border carries its colour; the theme's line width applies.
    let bottom = whole.bottom.as_ref().expect("bottom via lnRef");
    assert_eq!(bottom.color, Some(Color::Scheme("accent1".into())));
    assert_eq!(bottom.width_emu, None);
    assert!(whole.right.is_none());

    let first_row = style.first_row.as_ref().expect("firstRow");
    assert_eq!(first_row.bold, Some(true));
    assert_eq!(first_row.text_color, Some(Color::Scheme("lt1".into())));
    // A fillRef fill reads as a solid of its colour.
    assert_eq!(
        first_row.fill,
        Some(Fill::Solid(Color::Scheme("accent1".into())))
    );
    assert!(style.band1_h.is_none());
}
