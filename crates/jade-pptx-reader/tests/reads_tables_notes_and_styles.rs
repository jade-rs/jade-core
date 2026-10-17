//! The third reader slice: what real decks carry that the first two did not —
//! tables in graphic frames, speaker notes, paragraph spacing, East Asian
//! typefaces, shape style references, colour transforms, hyperlinks, and the
//! master/layout text styles a placeholder inherits from.
//!
//! Every fixture here is modelled on XML PowerPoint or Google Slides actually
//! writes, so a passing test means a real deck of that shape reads.

use std::io::Write;

use jade_pptx_reader::{
    read_deck, Autofit, Bullet, Color, Deck, Fill, ShapeItem, Spacing, TextAlign, TextAnchor,
};

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const PRESENTATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>
  <p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst>
  <p:sldSz cx="12192000" cy="6858000"/>
  <p:defaultTextStyle>
    <a:lvl1pPr><a:defRPr sz="1800"/></a:lvl1pPr>
  </p:defaultTextStyle>
</p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#;

const SLIDE_MASTER: &str = r#"<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:bg><p:bgPr><a:solidFill><a:schemeClr val="bg2"/></a:solidFill></p:bgPr></p:bg>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr/>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title Placeholder 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="10515600" cy="1325563"/></a:xfrm></p:spPr>
        <p:txBody><a:bodyPr anchor="ctr"/><a:lstStyle/><a:p><a:r><a:t>Click to edit</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Body Placeholder 2"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="838200" y="1825625"/><a:ext cx="10515600" cy="4351338"/></a:xfrm></p:spPr>
        <p:txBody><a:bodyPr/><a:lstStyle><a:lvl1pPr><a:defRPr b="1"/></a:lvl1pPr></a:lstStyle><a:p><a:r><a:t>Click to edit</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="4" name="Logo"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="ABCDEF"/></a:solidFill></p:spPr>
      </p:sp>
    </p:spTree>
  </p:cSld>
  <p:clrMap bg1="dk1" tx1="lt1" bg2="dk2" tx2="lt2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>
  <p:txStyles>
    <p:titleStyle>
      <a:lvl1pPr algn="l"><a:lnSpc><a:spcPct val="90000"/></a:lnSpc><a:buNone/><a:defRPr sz="4400"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mj-lt"/><a:ea typeface="+mj-ea"/></a:defRPr></a:lvl1pPr>
    </p:titleStyle>
    <p:bodyStyle>
      <a:lvl1pPr marL="228600" indent="-228600"><a:spcBef><a:spcPts val="1000"/></a:spcBef><a:buFont typeface="Arial"/><a:buChar char="•"/><a:defRPr sz="2800"><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl1pPr>
      <a:lvl2pPr marL="685800" indent="-228600"><a:buChar char="–"/><a:defRPr sz="2400"/></a:lvl2pPr>
    </p:bodyStyle>
    <p:otherStyle>
      <a:lvl1pPr><a:defRPr sz="1400"/></a:lvl1pPr>
    </p:otherStyle>
  </p:txStyles>
</p:sldMaster>"#;

const SLIDE_MASTER_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
</Relationships>"#;

const SLIDE_LAYOUT: &str = r#"<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="obj" showMasterSp="0">
  <p:cSld name="Title and Content">
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr/>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:lstStyle><a:lvl1pPr algn="ctr"><a:defRPr sz="4000"/></a:lvl1pPr></a:lstStyle><a:p><a:r><a:t>Click to edit</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Content Placeholder 2"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Click to edit</a:t></a:r></a:p></p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sldLayout>"#;

const SLIDE_LAYOUT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>
</Relationships>"#;

const THEME: &str = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office">
  <a:themeElements>
    <a:clrScheme name="Office">
      <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
      <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
      <a:dk2><a:srgbClr val="44546A"/></a:dk2>
      <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
      <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
      <a:accent3><a:srgbClr val="A5A5A5"/></a:accent3>
      <a:accent4><a:srgbClr val="FFC000"/></a:accent4>
      <a:accent5><a:srgbClr val="5B9BD5"/></a:accent5>
      <a:accent6><a:srgbClr val="70AD47"/></a:accent6>
      <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
      <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
    </a:clrScheme>
    <a:fontScheme name="Office">
      <a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface="Yu Gothic Light"/><a:cs typeface=""/></a:majorFont>
      <a:minorFont><a:latin typeface="Calibri"/><a:ea typeface="Yu Gothic"/><a:cs typeface=""/></a:minorFont>
    </a:fontScheme>
  </a:themeElements>
</a:theme>"#;

const SLIDE_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide1.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/mcp" TargetMode="External"/>
</Relationships>"#;

const NOTES_SLIDE: &str = r#"<p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Slide Image Placeholder 1"/><p:cNvSpPr/><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr>
      <p:spPr/>
    </p:sp>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="3" name="Notes Placeholder 2"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
      <p:spPr/>
      <p:txBody><a:bodyPr/><a:lstStyle/>
        <a:p><a:r><a:t>【13:00 開始】</a:t></a:r></a:p>
        <a:p><a:r><a:t>皆さま、始めます。</a:t></a:r></a:p>
      </p:txBody>
    </p:sp>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="4" name="Slide Number Placeholder 3"/><p:cNvSpPr/><p:nvPr><p:ph type="sldNum" idx="10"/></p:nvPr></p:nvSpPr>
      <p:spPr/>
      <p:txBody><a:bodyPr/><a:p><a:fld type="slidenum"><a:t>1</a:t></a:fld></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:notes>"#;

const SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" showMasterSp="0">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr/>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Styled 1"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr>
          <a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
          <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
          <a:ln w="19050"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:prstDash val="dash"/></a:ln>
        </p:spPr>
        <p:style>
          <a:lnRef idx="2"><a:schemeClr val="accent5"/></a:lnRef>
          <a:fillRef idx="1"><a:schemeClr val="accent1"><a:lumMod val="75000"/><a:lumOff val="25000"/></a:schemeClr></a:fillRef>
          <a:effectRef idx="0"><a:schemeClr val="accent5"/></a:effectRef>
          <a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef>
        </p:style>
        <p:txBody>
          <a:bodyPr wrap="none" anchor="ctr"><a:normAutofit fontScale="85000" lnSpcReduction="10000"/></a:bodyPr>
          <a:lstStyle/>
          <a:p>
            <a:pPr marL="342900" indent="-342900" algn="ctr">
              <a:lnSpc><a:spcPct val="150000"/></a:lnSpc>
              <a:spcBef><a:spcPts val="600"/></a:spcBef>
              <a:spcAft><a:spcPct val="20000"/></a:spcAft>
              <a:buSzPct val="80000"/>
              <a:buFont typeface="Wingdings"/>
              <a:buChar char="§"/>
            </a:pPr>
            <a:r>
              <a:rPr lang="ja-JP" sz="1200" strike="sngStrike">
                <a:solidFill><a:srgbClr val="4472C4"><a:alpha val="50000"/></a:srgbClr></a:solidFill>
                <a:latin typeface="Yu Gothic"/><a:ea typeface="Yu Gothic Medium"/>
                <a:hlinkClick r:id="rId3"/>
              </a:rPr>
              <a:t>対象</a:t>
            </a:r>
            <a:endParaRPr sz="1200"/>
          </a:p>
          <a:p><a:endParaRPr sz="3600"/></a:p>
        </p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="No outline"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr>
          <a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
          <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
          <a:ln><a:noFill/></a:ln>
        </p:spPr>
        <p:txBody><a:bodyPr><a:spAutoFit/></a:bodyPr><a:p><a:r><a:t>x</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="4" name="Title 3"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Inherits everything</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="5" name="Content 4"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:lstStyle/>
          <a:p><a:r><a:t>Level one</a:t></a:r></a:p>
          <a:p><a:pPr lvl="1"/><a:r><a:t>Level two</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
      <p:graphicFrame>
        <p:nvGraphicFramePr><p:cNvPr id="6" name="Table 5"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr>
        <p:xfrm><a:off x="914400" y="1828800"/><a:ext cx="2743200" cy="768096"/></p:xfrm>
        <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
          <a:tbl>
            <a:tblPr firstRow="1" bandRow="1"/>
            <a:tblGrid><a:gridCol w="914400"/><a:gridCol w="1828800"/></a:tblGrid>
            <a:tr h="384048">
              <a:tc>
                <a:txBody><a:bodyPr/><a:p><a:pPr algn="ctr"/><a:r><a:rPr sz="1200" b="1"/><a:t>Header A</a:t></a:r></a:p></a:txBody>
                <a:tcPr anchor="ctr" marL="45720">
                  <a:lnL w="10160"><a:solidFill><a:srgbClr val="D9D9D9"/></a:solidFill></a:lnL>
                  <a:lnB w="20320"><a:solidFill><a:srgbClr val="000000"/></a:solidFill><a:prstDash val="sysDash"/></a:lnB>
                  <a:solidFill><a:srgbClr val="1F2937"/></a:solidFill>
                </a:tcPr>
              </a:tc>
              <a:tc>
                <a:txBody><a:bodyPr/><a:p><a:r><a:t>Header B</a:t></a:r></a:p></a:txBody>
                <a:tcPr><a:noFill/></a:tcPr>
              </a:tc>
            </a:tr>
            <a:tr h="384048">
              <a:tc gridSpan="2">
                <a:txBody><a:bodyPr/><a:p><a:r><a:t>Spans both</a:t></a:r></a:p></a:txBody>
                <a:tcPr/>
              </a:tc>
              <a:tc hMerge="1"><a:txBody><a:bodyPr/><a:p><a:endParaRPr/></a:p></a:txBody><a:tcPr/></a:tc>
            </a:tr>
          </a:tbl>
        </a:graphicData></a:graphic>
      </p:graphicFrame>
      <mc:AlternateContent>
        <mc:Choice Requires="p14">
          <p:sp>
            <p:nvSpPr><p:cNvPr id="7" name="Choice only"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
            <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1" cy="1"/></a:xfrm></p:spPr>
          </p:sp>
        </mc:Choice>
        <mc:Fallback>
          <p:sp>
            <p:nvSpPr><p:cNvPr id="8" name="Fallback shape"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
            <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1" cy="1"/></a:xfrm></p:spPr>
          </p:sp>
        </mc:Fallback>
      </mc:AlternateContent>
    </p:spTree>
  </p:cSld>
</p:sld>"#;

fn deck_bytes() -> Vec<u8> {
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
            ("ppt/slideLayouts/slideLayout1.xml", SLIDE_LAYOUT),
            (
                "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
                SLIDE_LAYOUT_RELS,
            ),
            ("ppt/theme/theme1.xml", THEME),
            ("ppt/slides/slide1.xml", SLIDE),
            ("ppt/slides/_rels/slide1.xml.rels", SLIDE_RELS),
            ("ppt/notesSlides/notesSlide1.xml", NOTES_SLIDE),
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

fn deck() -> Deck {
    read_deck(&deck_bytes()).expect("read")
}

fn shape_named<'a>(deck: &'a Deck, name: &str) -> &'a jade_pptx_reader::Shape {
    deck.slides[0]
        .content
        .shapes
        .iter()
        .find_map(|item| match item {
            ShapeItem::Shape(shape) if shape.name == name => Some(shape),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no shape named {name}"))
}

#[test]
fn a_table_in_a_graphic_frame_reads_its_grid_rows_and_cells() {
    let deck = deck();
    let table = deck.slides[0]
        .content
        .shapes
        .iter()
        .find_map(|item| match item {
            ShapeItem::Table(table) => Some(table),
            _ => None,
        })
        .expect("the graphic frame reads as a table");

    assert_eq!(table.name, "Table 5");
    let transform = table.transform.expect("p:xfrm");
    assert_eq!(transform.offset_emu, (914_400, 1_828_800));
    assert_eq!(transform.extent_emu, (2_743_200, 768_096));
    assert_eq!(table.column_widths_emu, vec![914_400, 1_828_800]);
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0].height_emu, 384_048);

    let header = &table.rows[0].cells[0];
    let text = header.text.as_ref().expect("cell text");
    assert_eq!(text.paragraphs[0].runs[0].text, "Header A");
    assert!(text.paragraphs[0].runs[0].bold);
    assert_eq!(header.fill, Some(Fill::Solid(Color::Srgb("1F2937".into()))));
    assert_eq!(header.anchor, Some(TextAnchor::Center));
    assert_eq!(header.insets.left_emu, Some(45_720));
    let left = header.borders.left.as_ref().expect("lnL");
    assert_eq!(left.width_emu, Some(10_160));
    assert_eq!(left.color, Some(Color::Srgb("D9D9D9".into())));
    let bottom = header.borders.bottom.as_ref().expect("lnB");
    assert_eq!(bottom.dash.as_deref(), Some("sysDash"));
    assert!(header.borders.right.is_none());

    assert_eq!(table.rows[0].cells[1].fill, Some(Fill::None));

    let spanning = &table.rows[1].cells[0];
    assert_eq!(spanning.grid_span, 2);
    assert!(!spanning.h_merge);
    let merged = &table.rows[1].cells[1];
    assert!(merged.h_merge);
    assert_eq!(merged.grid_span, 1);
}

#[test]
fn speaker_notes_come_back_as_the_notes_placeholder_text() {
    let deck = deck();
    assert_eq!(
        deck.slides[0].notes.as_deref(),
        Some("【13:00 開始】\n皆さま、始めます。")
    );
}

#[test]
fn paragraph_spacing_margins_and_bullet_styling_are_read() {
    let deck = deck();
    let paragraph = &shape_named(&deck, "Styled 1")
        .text
        .as_ref()
        .expect("text")
        .paragraphs[0];
    assert_eq!(paragraph.line_spacing, Some(Spacing::Percent(1.5)));
    assert_eq!(paragraph.space_before, Some(Spacing::Points(6.0)));
    assert_eq!(paragraph.space_after, Some(Spacing::Percent(0.2)));
    assert_eq!(paragraph.margin_left_emu, Some(342_900));
    assert_eq!(paragraph.indent_emu, Some(-342_900));
    assert_eq!(paragraph.align, Some(TextAlign::Center));
    assert_eq!(paragraph.bullet, Some(Bullet::Char("§".into())));
    assert_eq!(paragraph.bullet_font.as_deref(), Some("Wingdings"));
    assert_eq!(paragraph.bullet_size_percent, Some(0.8));
    assert_eq!(paragraph.end_size_pt, Some(12.0));
}

#[test]
fn an_empty_paragraph_keeps_its_end_run_size() {
    let deck = deck();
    let paragraph = &shape_named(&deck, "Styled 1")
        .text
        .as_ref()
        .expect("text")
        .paragraphs[1];
    assert!(paragraph.runs.is_empty());
    assert_eq!(paragraph.end_size_pt, Some(36.0));
}

#[test]
fn runs_carry_east_asian_typeface_strike_alpha_and_hyperlink() {
    let deck = deck();
    let run = &shape_named(&deck, "Styled 1")
        .text
        .as_ref()
        .expect("text")
        .paragraphs[0]
        .runs[0];
    assert_eq!(run.font.as_deref(), Some("Yu Gothic"));
    assert_eq!(run.east_asian_font.as_deref(), Some("Yu Gothic Medium"));
    assert!(run.strike);
    assert_eq!(
        run.color,
        Some(Color::Adjusted {
            base: Box::new(Color::Srgb("4472C4".into())),
            alpha: Some(0.5),
            lum_mod: None,
            lum_off: None,
            shade: None,
            tint: None,
        })
    );
    let link = run.hyperlink.as_ref().expect("hlinkClick");
    assert_eq!(link.rel_id, "rId3");
    assert_eq!(link.target.as_deref(), Some("https://example.com/mcp"));
}

#[test]
fn body_properties_carry_wrap_and_autofit() {
    let deck = deck();
    let styled = shape_named(&deck, "Styled 1").text.as_ref().expect("text");
    assert!(!styled.wrap);
    assert_eq!(
        styled.autofit,
        Autofit::Normal {
            font_scale: 0.85,
            line_spacing_reduction: 0.1
        }
    );
    let auto = shape_named(&deck, "No outline")
        .text
        .as_ref()
        .expect("text");
    assert!(auto.wrap);
    assert_eq!(auto.autofit, Autofit::Shape);
}

#[test]
fn a_shape_style_reference_and_dashed_outline_are_read() {
    let deck = deck();
    let shape = shape_named(&deck, "Styled 1");
    let style = shape.style.as_ref().expect("p:style");
    assert_eq!(style.line_color, Some(Color::Scheme("accent5".into())));
    assert_eq!(
        style.fill_color,
        Some(Color::Adjusted {
            base: Box::new(Color::Scheme("accent1".into())),
            alpha: None,
            lum_mod: Some(0.75),
            lum_off: Some(0.25),
            shade: None,
            tint: None,
        })
    );
    assert_eq!(style.font_color, Some(Color::Scheme("lt1".into())));
    assert_eq!(style.fill_index, 1);
    assert_eq!(style.line_index, 2);

    let outline = shape.outline.as_ref().expect("a:ln");
    assert_eq!(outline.dash.as_deref(), Some("dash"));
    assert!(!outline.no_fill);
    assert_eq!(outline.width_emu, Some(19_050));

    let none = shape_named(&deck, "No outline")
        .outline
        .as_ref()
        .expect("a:ln with noFill still reads");
    assert!(none.no_fill);
}

#[test]
fn a_fallback_shape_inside_alternate_content_is_read_and_the_choice_is_not() {
    let deck = deck();
    let names: Vec<&str> = deck.slides[0]
        .content
        .shapes
        .iter()
        .filter_map(|item| match item {
            ShapeItem::Shape(shape) => Some(shape.name.as_str()),
            _ => None,
        })
        .collect();
    assert!(names.contains(&"Fallback shape"), "{names:?}");
    assert!(!names.contains(&"Choice only"), "{names:?}");
}

#[test]
fn master_and_layout_carry_show_master_shapes_and_backgrounds() {
    let deck = deck();
    let slide = &deck.slides[0];
    assert!(!slide.content.show_master_shapes);
    let layout = slide.layout_content.as_ref().expect("layout");
    assert!(!layout.show_master_shapes);
    let master = slide.master_content.as_ref().expect("master");
    assert!(master.show_master_shapes);
    assert_eq!(
        master.background,
        Some(Fill::Solid(Color::Scheme("bg2".into())))
    );
}

#[test]
fn the_master_color_map_redirects_the_background_and_text_aliases() {
    // This master maps bg1→dk1 and tx1→lt1: a dark-background theme.
    let deck = deck();
    assert_eq!(deck.theme_colors.get("bg1"), Some("000000"));
    assert_eq!(deck.theme_colors.get("tx1"), Some("FFFFFF"));
    assert_eq!(deck.theme_colors.get("bg2"), Some("44546A"));
    assert_eq!(deck.theme_colors.get("accent1"), Some("4472C4"));
}

#[test]
fn theme_fonts_resolve_the_major_and_minor_placeholders() {
    let deck = deck();
    assert_eq!(deck.theme_fonts.resolve("+mj-lt"), Some("Calibri Light"));
    assert_eq!(deck.theme_fonts.resolve("+mn-lt"), Some("Calibri"));
    assert_eq!(deck.theme_fonts.resolve("+mj-ea"), Some("Yu Gothic Light"));
    assert_eq!(deck.theme_fonts.resolve("+mn-ea"), Some("Yu Gothic"));
    assert_eq!(deck.theme_fonts.resolve("Meiryo"), None);
}

#[test]
fn master_text_styles_are_read_per_level() {
    let deck = deck();
    let master = deck.slides[0].master_content.as_ref().expect("master");
    let styles = master.text_styles.as_ref().expect("p:txStyles");

    let title = styles.title.level(0).expect("titleStyle lvl1");
    assert_eq!(title.default_run.size_pt, Some(44.0));
    assert_eq!(title.default_run.font.as_deref(), Some("+mj-lt"));
    assert_eq!(title.default_run.east_asian_font.as_deref(), Some("+mj-ea"));
    assert_eq!(title.default_run.color, Some(Color::Scheme("tx1".into())));
    assert_eq!(title.line_spacing, Some(Spacing::Percent(0.9)));
    assert_eq!(title.bullet, Some(Bullet::None));
    assert_eq!(title.align, Some(TextAlign::Left));

    let body1 = styles.body.level(0).expect("bodyStyle lvl1");
    assert_eq!(body1.margin_left_emu, Some(228_600));
    assert_eq!(body1.indent_emu, Some(-228_600));
    assert_eq!(body1.bullet, Some(Bullet::Char("•".into())));
    assert_eq!(body1.bullet_font.as_deref(), Some("Arial"));
    assert_eq!(body1.space_before, Some(Spacing::Points(10.0)));
    assert_eq!(body1.default_run.size_pt, Some(28.0));
    let body2 = styles.body.level(1).expect("bodyStyle lvl2");
    assert_eq!(body2.default_run.size_pt, Some(24.0));
    assert_eq!(body2.bullet, Some(Bullet::Char("–".into())));
    assert!(styles.body.level(2).is_none());

    assert_eq!(
        styles
            .other
            .level(0)
            .expect("otherStyle")
            .default_run
            .size_pt,
        Some(14.0)
    );
}

#[test]
fn placeholder_list_styles_are_read_on_every_tree() {
    let deck = deck();
    let slide = &deck.slides[0];
    let layout = slide.layout_content.as_ref().expect("layout");
    let layout_title = match &layout.shapes[0] {
        ShapeItem::Shape(shape) => shape,
        other => panic!("{other:?}"),
    };
    let list_style = layout_title
        .text
        .as_ref()
        .expect("text")
        .list_style
        .as_ref()
        .expect("lstStyle");
    let level = list_style.level(0).expect("lvl1pPr");
    assert_eq!(level.align, Some(TextAlign::Center));
    assert_eq!(level.default_run.size_pt, Some(40.0));

    let master = slide.master_content.as_ref().expect("master");
    let master_body = match &master.shapes[1] {
        ShapeItem::Shape(shape) => shape,
        other => panic!("{other:?}"),
    };
    let level = master_body
        .text
        .as_ref()
        .expect("text")
        .list_style
        .as_ref()
        .expect("lstStyle")
        .level(0)
        .expect("lvl1pPr");
    assert_eq!(level.default_run.bold, Some(true));
}

#[test]
fn the_presentation_default_text_style_is_read() {
    let deck = deck();
    let level = deck
        .default_text_style
        .level(0)
        .expect("defaultTextStyle lvl1");
    assert_eq!(level.default_run.size_pt, Some(18.0));
}
