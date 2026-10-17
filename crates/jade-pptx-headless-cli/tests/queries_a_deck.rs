//! What an agent or a script sees when it asks about a deck.

use std::io::Write;

use jade_pptx_headless_api::{DeckSummary, ShapeKind, SlideView, TextMatch};
use jade_pptx_headless_cli::{deck_summary, deck_text, find_text, slide_view};

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const PRESENTATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst>
  <p:sldSz cx="12192000" cy="6858000"/>
</p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide2.xml"/>
</Relationships>"#;

const SLIDE_ONE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
      <p:txBody><a:bodyPr/><a:p><a:r><a:t>Quarterly Review</a:t></a:r></a:p></p:txBody>
    </p:sp>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="3" name="Body 2"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="0" y="1000000"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom></p:spPr>
      <p:txBody><a:bodyPr/>
        <a:p><a:r><a:t>Revenue grew</a:t></a:r></a:p>
        <a:p><a:r><a:t>Costs fell</a:t></a:r></a:p>
      </p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:sld>"#;

const SLIDE_TWO: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:grpSp>
      <p:nvGrpSpPr><p:cNvPr id="4" name="Group 1"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/><a:chOff x="0" y="0"/><a:chExt cx="914400" cy="914400"/></a:xfrm></p:grpSpPr>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="5" name="Grouped 2"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="457200" cy="457200"/></a:xfrm><a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom></p:spPr>
        <p:txBody><a:bodyPr/><a:p><a:r><a:t>Nested note</a:t></a:r></a:p></p:txBody>
      </p:sp>
    </p:grpSp>
    <p:pic>
      <p:nvPicPr><p:cNvPr id="6" name="Chart.png"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
      <p:blipFill><a:blip r:embed="rId9"/></p:blipFill>
      <p:spPr><a:xfrm><a:off x="0" y="500000"/><a:ext cx="914400" cy="914400"/></a:xfrm></p:spPr>
    </p:pic>
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
    buffer
}

#[test]
fn the_summary_reports_slides_titles_and_shape_counts() {
    let summary: DeckSummary = deck_summary(&deck_bytes()).expect("summary");
    assert_eq!(summary.slide_count, 2);
    assert_eq!(summary.width_px, 1280.0);
    assert_eq!(summary.height_px, 720.0);

    assert_eq!(summary.slides[0].index, 1);
    assert_eq!(summary.slides[0].title.as_deref(), Some("Quarterly Review"));
    assert_eq!(summary.slides[0].shape_count, 2);

    // Slide 2 has no title placeholder; the group and picture still count.
    assert_eq!(summary.slides[1].title, None);
    assert_eq!(summary.slides[1].shape_count, 2);
}

#[test]
fn a_slide_view_lists_shapes_with_their_text() {
    let view: SlideView = slide_view(&deck_bytes(), 1).expect("slide view");
    assert_eq!(view.index, 1);
    assert_eq!(view.shapes.len(), 2);
    assert_eq!(view.shapes[0].name, "Title 1");
    assert_eq!(view.shapes[0].kind, ShapeKind::Shape);
    assert_eq!(view.shapes[0].geometry.as_deref(), Some("rect"));
    assert_eq!(view.shapes[0].text.as_deref(), Some("Quarterly Review"));
    // Paragraphs join with newlines.
    assert_eq!(
        view.shapes[1].text.as_deref(),
        Some("Revenue grew\nCosts fell")
    );
}

#[test]
fn a_group_nests_its_children_in_the_view() {
    let view = slide_view(&deck_bytes(), 2).expect("slide view");
    assert_eq!(view.shapes[0].kind, ShapeKind::Group);
    assert_eq!(view.shapes[0].children.len(), 1);
    assert_eq!(
        view.shapes[0].children[0].text.as_deref(),
        Some("Nested note")
    );
    assert_eq!(view.shapes[1].kind, ShapeKind::Picture);
    assert_eq!(view.shapes[1].name, "Chart.png");
}

#[test]
fn an_out_of_range_slide_index_is_an_error() {
    assert!(slide_view(&deck_bytes(), 0).is_err());
    assert!(slide_view(&deck_bytes(), 3).is_err());
}

#[test]
fn deck_text_flattens_every_slide_in_order() {
    let text = deck_text(&deck_bytes()).expect("text");
    assert_eq!(
        text,
        "## Slide 1\nQuarterly Review\nRevenue grew\nCosts fell\n\n## Slide 2\nNested note\n"
    );
}

#[test]
fn find_reports_slide_shape_and_matched_text() {
    let matches: Vec<TextMatch> = find_text(&deck_bytes(), "note", false).expect("find");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].slide, 2);
    assert_eq!(matches[0].shape, "Grouped 2");
    assert_eq!(matches[0].text, "Nested note");

    // Case-insensitive by default, sensitive on request.
    assert_eq!(
        find_text(&deck_bytes(), "NOTE", false).expect("find").len(),
        1
    );
    assert!(find_text(&deck_bytes(), "NOTE", true)
        .expect("find")
        .is_empty());
}

#[test]
fn a_picture_reports_its_image_part_and_media_returns_the_bytes() {
    let view = slide_view(&deck_bytes(), 2).expect("slide view");
    let picture = &view.shapes[1];
    assert_eq!(picture.kind, ShapeKind::Picture);
    // The fixture's slide has no rels, so the blip's part is unresolved.
    assert_eq!(picture.image_part, None);

    // A package with a media part: the bytes come back with a MIME type.
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", SLIDE_ONE),
            ("ppt/slides/slide2.xml", SLIDE_TWO),
            ("ppt/media/image1.png", "PNG-BYTES"),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    let (bytes, content_type) =
        jade_pptx_headless_cli::media(&buffer, "ppt/media/image1.png").expect("media");
    assert_eq!(bytes, b"PNG-BYTES");
    assert_eq!(content_type, "image/png");
    let (_, content_type) =
        jade_pptx_headless_cli::media(&buffer, "/ppt/media/image1.png").expect("leading slash");
    assert_eq!(content_type, "image/png");
    assert!(jade_pptx_headless_cli::media(&buffer, "ppt/media/missing.png").is_err());
}

#[test]
fn a_hidden_slide_is_flagged_in_the_summary() {
    let hidden = SLIDE_ONE.replace(
        r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">"#,
        r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" show="0">"#,
    );
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", hidden.as_str()),
            ("ppt/slides/slide2.xml", SLIDE_TWO),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    let summary = deck_summary(&buffer).expect("summary");
    assert!(summary.slides[0].hidden);
    assert!(!summary.slides[1].hidden);
}

const CHART_SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:graphicFrame>
      <p:nvGraphicFramePr><p:cNvPr id="3" name="Chart 2"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
      <p:xfrm><a:off x="914400" y="914400"/><a:ext cx="4572000" cy="2743200"/></p:xfrm>
      <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rId6"/></a:graphicData></a:graphic>
    </p:graphicFrame>
  </p:spTree></p:cSld>
</p:sld>"#;

const CHART_SLIDE_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>"#;

const CHART_PART: &str = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart>
  <c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Quarterly Revenue</a:t></a:r></a:p></c:rich></c:tx></c:title>
  <c:plotArea><c:lineChart><c:ser><c:tx><c:v>Revenue</c:v></c:tx>
    <c:cat><c:strLit><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strLit></c:cat>
    <c:val><c:numLit><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>12.5</c:v></c:pt></c:numLit></c:val>
  </c:ser></c:lineChart></c:plotArea>
</c:chart></c:chartSpace>"#;

fn chart_deck_bytes() -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", CHART_SLIDE),
            ("ppt/slides/_rels/slide1.xml.rels", CHART_SLIDE_RELS),
            ("ppt/slides/slide2.xml", SLIDE_TWO),
            ("ppt/charts/chart1.xml", CHART_PART),
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
fn a_chart_shows_its_kind_title_and_series_in_the_view_text_and_find() {
    let bytes = chart_deck_bytes();
    let view = slide_view(&bytes, 1).expect("view");
    let chart = &view.shapes[0];
    assert_eq!(chart.kind, ShapeKind::Chart);
    assert_eq!(chart.name, "Chart 2");
    assert_eq!(chart.text.as_deref(), Some("Quarterly Revenue"));
    let data = chart.chart.as_ref().expect("chart data");
    assert_eq!(data.kind.as_deref(), Some("lineChart"));
    assert_eq!(data.title.as_deref(), Some("Quarterly Revenue"));
    assert_eq!(data.series.len(), 1);
    assert_eq!(data.series[0].name.as_deref(), Some("Revenue"));
    assert_eq!(data.series[0].categories, vec!["Q1", "Q2"]);
    assert_eq!(data.series[0].values, vec![Some(10.0), Some(12.5)]);
    let rect = chart.rect.expect("placed");
    assert!((rect.x - 96.0).abs() < 1e-9);

    let text = deck_text(&bytes).expect("text");
    assert!(
        text.starts_with("## Slide 1\nQuarterly Revenue\nRevenue: Q1=10, Q2=12.5\n"),
        "{text}"
    );

    let matches = find_text(&bytes, "revenue", false).expect("find");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].shape, "Chart 2");
    assert!(matches[0].text.contains("Quarterly Revenue"));
}
