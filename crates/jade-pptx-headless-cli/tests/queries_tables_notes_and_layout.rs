//! What an agent sees of a deck that carries the things real decks carry:
//! a table, speaker notes, a placeholder positioned by its layout, a
//! hyperlink — and what the layout engine says would go wrong on screen.

use std::io::Write;

use jade_pptx_headless_api::{IssueKind, ShapeKind};
use jade_pptx_headless_cli::{
    check_layout, deck_summary, deck_text, find_text, slide_layout, slide_view,
};

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const PRESENTATION: &str = r#"<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>
  <p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst>
  <p:sldSz cx="12192000" cy="6858000"/>
  <p:defaultTextStyle><a:lvl1pPr><a:defRPr sz="1800"/></a:lvl1pPr></p:defaultTextStyle>
</p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide2.xml"/>
</Relationships>"#;

const SLIDE_MASTER: &str = r#"<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Title Placeholder 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="10515600" cy="1325563"/></a:xfrm></p:spPr>
      <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Click to edit</a:t></a:r></a:p></p:txBody>
    </p:sp>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="3" name="Master Logo"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="ABCDEF"/></a:solidFill></p:spPr>
    </p:sp>
  </p:spTree></p:cSld>
  <p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>
  <p:txStyles>
    <p:titleStyle><a:lvl1pPr><a:defRPr sz="4400"/></a:lvl1pPr></p:titleStyle>
    <p:bodyStyle><a:lvl1pPr><a:defRPr sz="2800"/></a:lvl1pPr></p:bodyStyle>
    <p:otherStyle><a:lvl1pPr><a:defRPr sz="1800"/></a:lvl1pPr></p:otherStyle>
  </p:txStyles>
</p:sldMaster>"#;

const SLIDE_MASTER_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
</Relationships>"#;

const SLIDE_LAYOUT: &str = r#"<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="title">
  <p:cSld name="Title Slide"><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="ctrTitle"/></p:nvPr></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="1524000" y="1122363"/><a:ext cx="9144000" cy="2387600"/></a:xfrm></p:spPr>
      <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Click to edit</a:t></a:r></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:sldLayout>"#;

const SLIDE_LAYOUT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>
</Relationships>"#;

const SLIDE_ONE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="ctrTitle"/></p:nvPr></p:nvSpPr>
      <p:spPr/>
      <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Quarterly Review</a:t></a:r></a:p></p:txBody>
    </p:sp>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="3" name="Tiny box"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="914400" y="4572000"/><a:ext cx="1828800" cy="228600"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
      <p:txBody><a:bodyPr/><a:p><a:r><a:rPr sz="1800"><a:hlinkClick r:id="rId2"/></a:rPr><a:t>This sentence is far too long for its little box</a:t></a:r></a:p></p:txBody>
    </p:sp>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="4" name="Off slide"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="13000000" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom></p:spPr>
    </p:sp>
    <p:graphicFrame>
      <p:nvGraphicFramePr><p:cNvPr id="5" name="Table 4"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
      <p:xfrm><a:off x="914400" y="2743200"/><a:ext cx="2743200" cy="768096"/></p:xfrm>
      <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
        <a:tbl>
          <a:tblGrid><a:gridCol w="914400"/><a:gridCol w="1828800"/></a:tblGrid>
          <a:tr h="384048">
            <a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>対象</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>
            <a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>Revenue</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>
          </a:tr>
          <a:tr h="384048">
            <a:tc gridSpan="2"><a:txBody><a:bodyPr/><a:p><a:r><a:t>Spans both</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>
            <a:tc hMerge="1"><a:txBody><a:bodyPr/><a:p><a:endParaRPr/></a:p></a:txBody><a:tcPr/></a:tc>
          </a:tr>
        </a:tbl>
      </a:graphicData></a:graphic>
    </p:graphicFrame>
  </p:spTree></p:cSld>
</p:sld>"#;

const SLIDE_ONE_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/report" TargetMode="External"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide1.xml"/>
</Relationships>"#;

const NOTES_ONE: &str = r#"<p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="3" name="Notes Placeholder 2"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
      <p:spPr/>
      <p:txBody><a:bodyPr/><a:p><a:r><a:t>Open with the numbers.</a:t></a:r></a:p><a:p><a:r><a:t>Then pause.</a:t></a:r></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:notes>"#;

const SLIDE_TWO: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Fits"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="9144000" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
      <p:txBody><a:bodyPr/><a:p><a:r><a:t>Short</a:t></a:r></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
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
            ("ppt/slides/slide1.xml", SLIDE_ONE),
            ("ppt/slides/_rels/slide1.xml.rels", SLIDE_ONE_RELS),
            ("ppt/notesSlides/notesSlide1.xml", NOTES_ONE),
            ("ppt/slides/slide2.xml", SLIDE_TWO),
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
fn the_summary_counts_tables_and_reports_notes() {
    let summary = deck_summary(&deck_bytes()).expect("summary");
    assert_eq!(summary.slides[0].shape_count, 4);
    assert!(summary.slides[0].has_notes);
    assert!(!summary.slides[1].has_notes);
    assert_eq!(summary.slides[0].title.as_deref(), Some("Quarterly Review"));
}

#[test]
fn a_slide_view_places_shapes_reads_tables_notes_and_links() {
    let view = slide_view(&deck_bytes(), 1).expect("view");
    assert_eq!(
        view.notes.as_deref(),
        Some("Open with the numbers.\nThen pause.")
    );

    let title = &view.shapes[0];
    assert_eq!(title.id, Some(2));
    assert_eq!(title.placeholder.as_deref(), Some("ctrTitle"));
    // The title states no geometry; it lands where the layout puts it.
    let rect = title.rect.expect("inherited rect");
    assert!((rect.x - 160.0).abs() < 1e-9, "{rect:?}");
    assert!((rect.width - 960.0).abs() < 1e-9, "{rect:?}");

    let linked = &view.shapes[1];
    assert_eq!(linked.hyperlinks, vec!["https://example.com/report"]);
    let rect = linked.rect.expect("rect");
    assert!((rect.x - 96.0).abs() < 1e-9);
    assert!((rect.height - 24.0).abs() < 1e-9);

    let table = &view.shapes[3];
    assert_eq!(table.kind, ShapeKind::Table);
    assert_eq!(table.name, "Table 4");
    assert_eq!(
        table.rows,
        vec![
            vec!["対象".to_string(), "Revenue".to_string()],
            vec!["Spans both".to_string(), String::new()],
        ]
    );
    let rect = table.rect.expect("table rect is its cells' union");
    assert!((rect.x - 96.0).abs() < 1e-9, "{rect:?}");
    assert!((rect.width - 288.0).abs() < 1e-9, "{rect:?}");
}

#[test]
fn deck_text_includes_table_rows_and_quoted_notes() {
    let text = deck_text(&deck_bytes()).expect("text");
    assert_eq!(
        text,
        "## Slide 1\nQuarterly Review\nThis sentence is far too long for its little box\n対象\tRevenue\nSpans both\t\n> Open with the numbers.\n> Then pause.\n\n## Slide 2\nShort\n"
    );
}

#[test]
fn find_reaches_into_table_cells_and_notes() {
    let matches = find_text(&deck_bytes(), "revenue", false).expect("find");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].shape, "Table 4!B1");
    assert!(!matches[0].in_notes);

    let matches = find_text(&deck_bytes(), "pause", false).expect("find");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].shape, "Notes");
    assert!(matches[0].in_notes);
    assert_eq!(matches[0].slide, 1);
}

#[test]
fn the_layout_view_lists_master_shapes_first_and_reports_overflow() {
    let layout = slide_layout(&deck_bytes(), 1).expect("layout");
    assert_eq!(layout.width_px, 1280.0);
    assert_eq!(layout.shapes[0].name, "Master Logo");
    assert_eq!(layout.shapes[0].origin, "master");
    assert_eq!(layout.shapes[0].fill.as_deref(), Some("#ABCDEF"));

    let title = layout
        .shapes
        .iter()
        .find(|shape| shape.name == "Title 1")
        .expect("title");
    assert_eq!(title.origin, "slide");
    assert_eq!(title.lines, vec!["Quarterly Review"]);
    assert_eq!(title.text_overflow_px, 0.0);

    let tiny = layout
        .shapes
        .iter()
        .find(|shape| shape.name == "Tiny box")
        .expect("tiny box");
    assert!(tiny.lines.len() >= 2, "{:?}", tiny.lines);
    assert!(tiny.text_overflow_px > 20.0, "{}", tiny.text_overflow_px);

    let cell = layout
        .shapes
        .iter()
        .find(|shape| shape.name == "Table 4!A1")
        .expect("cell");
    assert_eq!(cell.lines, vec!["対象"]);
}

#[test]
fn check_reports_overflowing_text_and_shapes_off_the_slide() {
    let issues = check_layout(&deck_bytes(), None).expect("check");
    let kinds: Vec<(usize, &str, IssueKind)> = issues
        .iter()
        .map(|issue| (issue.slide, issue.shape.as_str(), issue.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (1, "Tiny box", IssueKind::TextOverflow),
            (1, "Off slide", IssueKind::OutsideSlide),
        ]
    );
    assert_eq!(issues[0].id, Some(3));
    assert!(issues[0].amount_px > 20.0);
    assert!(issues[0].detail.contains("px past the bottom"));
    assert!(issues[1].amount_px > 80.0, "{}", issues[1].amount_px);

    let second = check_layout(&deck_bytes(), Some(2)).expect("check slide 2");
    assert!(second.is_empty());
    assert!(check_layout(&deck_bytes(), Some(3)).is_err());
}

#[test]
fn check_reports_text_outside_a_diamonds_inner_region_even_when_it_fits_the_shape_frame() {
    let paragraphs = (0..4)
        .map(|_| "<a:p><a:r><a:rPr sz=\"1800\"/><a:t>line</a:t></a:r></a:p>")
        .collect::<String>();
    let slide = SLIDE_TWO
        .replace("prst=\"rect\"", "prst=\"diamond\"")
        .replace(
            "cx=\"9144000\" cy=\"914400\"",
            "cx=\"3048000\" cy=\"1905000\"",
        )
        .replace(
            "<a:bodyPr/><a:p><a:r><a:t>Short</a:t></a:r></a:p>",
            &format!("<a:bodyPr lIns=\"0\" rIns=\"0\" tIns=\"0\" bIns=\"0\"/>{paragraphs}"),
        );
    let mut package = jade_opc::OpcPackage::open(&deck_bytes()).unwrap();
    package.replace("ppt/slides/slide2.xml", slide.into_bytes());
    let bytes = package.write().unwrap();
    let view = slide_layout(&bytes, 2).unwrap();
    assert_eq!(view.shapes[0].rect.height, 200.0);
    assert_eq!(view.shapes[0].text_region.as_ref().unwrap().height, 100.0);
    assert!((view.shapes[0].text_overflow_px - 15.2).abs() < 1e-6);
    let issues = check_layout(&bytes, Some(2)).unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind, IssueKind::TextOverflow);
    assert!(issues[0]
        .detail
        .contains("geometry text region (100 px tall)"));
}

#[test]
fn a_large_glyph_that_only_overruns_its_line_box_is_not_an_issue() {
    // 40pt "1" centred in a box 59.5 px tall: the 64 px line box overruns by
    // 2 px, the glyph does not. PowerPoint shows it fine.
    let slide = SLIDE_TWO
        .replace(
            r#"<a:ext cx="9144000" cy="914400"/>"#,
            r#"<a:ext cx="1234440" cy="566928"/>"#,
        )
        .replace(
            "<a:bodyPr/><a:p><a:r><a:t>Short</a:t></a:r></a:p>",
            r#"<a:bodyPr anchor="ctr"/><a:p><a:r><a:rPr sz="4000" b="1"/><a:t>1</a:t></a:r></a:p>"#,
        );
    let mut bytes = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", SLIDE_TWO),
            ("ppt/slides/slide2.xml", slide.as_str()),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    let layout = slide_layout(&bytes, 2).expect("layout");
    assert!(
        layout.shapes[0].text_overflow_px > 1.0,
        "{}",
        layout.shapes[0].text_overflow_px
    );
    assert!(
        layout.shapes[0].text_overflow_px < 4.0,
        "{}",
        layout.shapes[0].text_overflow_px
    );
    assert!(check_layout(&bytes, Some(2)).expect("check").is_empty());
}

#[test]
fn check_reports_unwrapped_text_wider_than_its_shape() {
    let slide = SLIDE_TWO
        .replace(
            r#"<a:ext cx="9144000" cy="914400"/>"#,
            r#"<a:ext cx="914400" cy="914400"/>"#,
        )
        .replace(
            "<a:bodyPr/><a:p><a:r><a:t>Short</a:t></a:r></a:p>",
            r#"<a:bodyPr wrap="none"/><a:p><a:r><a:rPr sz="1800"/><a:t>this does not wrap and runs off</a:t></a:r></a:p>"#,
        );
    let mut bytes = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", SLIDE_TWO),
            ("ppt/slides/slide2.xml", slide.as_str()),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    let issues = check_layout(&bytes, Some(2)).expect("check");
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].kind, IssueKind::TextTooWide);
    assert!(issues[0].amount_px > 100.0, "{}", issues[0].amount_px);
    assert!(issues[0].detail.contains("right edge"));
}
#[test]
fn grouped_layout_reports_the_drawn_rotation_and_slide_bounds() {
    let bytes = include_bytes!("../../../public/fixtures/grouped-deck.pptx");
    let mut deck = jade_pptx_reader::read_deck(bytes).unwrap();
    let jade_pptx_reader::ShapeItem::Group(group) = &mut deck.slides[0].content.shapes[0] else {
        panic!("group");
    };
    group.transform.as_mut().unwrap().outer.rotation_60000ths = 90 * 60000;
    let view = jade_pptx_headless_cli::layout_view(&deck, 1).unwrap();
    let red = view
        .shapes
        .iter()
        .find(|shape| shape.name.starts_with("Picture"))
        .unwrap();
    assert!((red.rotation_deg - 120.0).abs() < 1e-8);
    let layout =
        jade_pptx_layout::layout_deck_slide(&deck, 0, &jade_pptx_layout::HeuristicMeasure).unwrap();
    let drawn = layout
        .shapes
        .iter()
        .find(|shape| shape.name == red.name)
        .unwrap();
    assert_eq!(red.rect.x, drawn.rect.x);
    assert_eq!(red.rect.y, drawn.rect.y);
    assert_eq!(red.rect.width, drawn.rect.width);
    assert_eq!(red.rect.height, drawn.rect.height);
}
