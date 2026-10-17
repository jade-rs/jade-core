//! What a slide holds beyond shapes and tables: SmartArt, read from the
//! drawing PowerPoint caches beside the diagram data; charts, read down to
//! their series so a caller can see the numbers; and shapes filled with a
//! picture or a pattern.

use std::io::Write;

use jade_pptx_reader::{read_deck, Color, Deck, Fill, ShapeItem};

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const PRESENTATION: &str = r#"<p:presentation xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/></p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#;

const SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:graphicFrame>
      <p:nvGraphicFramePr><p:cNvPr id="2" name="Diagram 1"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
      <p:xfrm><a:off x="914400" y="914400"/><a:ext cx="4572000" cy="2743200"/></p:xfrm>
      <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram">
        <dgm:relIds r:dm="rId5" r:lo="rId10" r:qs="rId11" r:cs="rId12"/>
      </a:graphicData></a:graphic>
    </p:graphicFrame>
    <p:graphicFrame>
      <p:nvGraphicFramePr><p:cNvPr id="3" name="Chart 2"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
      <p:xfrm><a:off x="6096000" y="914400"/><a:ext cx="4572000" cy="2743200"/></p:xfrm>
      <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
        <c:chart r:id="rId6"/>
      </a:graphicData></a:graphic>
    </p:graphicFrame>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="4" name="Photo box"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
      <p:spPr>
        <a:xfrm><a:off x="0" y="4000000"/><a:ext cx="914400" cy="914400"/></a:xfrm>
        <a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom>
        <a:blipFill><a:blip r:embed="rId7"/><a:stretch><a:fillRect/></a:stretch></a:blipFill>
      </p:spPr>
    </p:sp>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="5" name="Hatched"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
      <p:spPr>
        <a:xfrm><a:off x="1000000" y="4000000"/><a:ext cx="914400" cy="914400"/></a:xfrm>
        <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
        <a:pattFill prst="dkUpDiag"><a:fgClr><a:srgbClr val="000000"/></a:fgClr><a:bgClr><a:srgbClr val="FFFFFF"/></a:bgClr></a:pattFill>
      </p:spPr>
    </p:sp>
  </p:spTree></p:cSld>
</p:sld>"#;

const SLIDE_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData" Target="../diagrams/data1.xml"/>
  <Relationship Id="rId10" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramLayout" Target="../diagrams/layout1.xml"/>
  <Relationship Id="rId11" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramQuickStyle" Target="../diagrams/quickStyle1.xml"/>
  <Relationship Id="rId12" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramColors" Target="../diagrams/colors1.xml"/>
  <Relationship Id="rId9" Type="http://schemas.microsoft.com/office/2007/relationships/diagramDrawing" Target="../diagrams/drawing1.xml"/>
  <Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/>
  <Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
</Relationships>"#;

const DIAGRAM_DATA: &str = r#"<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram">
  <dgm:ptLst>
    <dgm:pt modelId="{A}" type="doc"><dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:p><a:endParaRPr/></a:p></dgm:t></dgm:pt>
    <dgm:pt modelId="{B}"><dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:p><a:r><a:t>Plan</a:t></a:r></a:p></dgm:t></dgm:pt>
    <dgm:pt modelId="{C}"><dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:p><a:r><a:t>Build</a:t></a:r></a:p></dgm:t></dgm:pt>
  </dgm:ptLst>
  <dgm:extLst><a:ext uri="http://schemas.microsoft.com/office/drawing/2008/diagram"><dsp:dataModelExt relId="rId9" minVer="http://schemas.openxmlformats.org/drawingml/2006/diagram"/></a:ext></dgm:extLst>
</dgm:dataModel>"#;

const DIAGRAM_DRAWING: &str = r#"<dsp:drawing xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <dsp:spTree>
    <dsp:nvGrpSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvGrpSpPr/></dsp:nvGrpSpPr>
    <dsp:grpSpPr/>
    <dsp:sp modelId="{B}">
      <dsp:nvSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvSpPr/></dsp:nvSpPr>
      <dsp:spPr>
        <a:xfrm><a:off x="0" y="0"/><a:ext cx="2000000" cy="1000000"/></a:xfrm>
        <a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom>
        <a:solidFill><a:schemeClr val="accent1"/></a:solidFill>
      </dsp:spPr>
      <dsp:style><a:lnRef idx="2"><a:schemeClr val="lt1"/></a:lnRef><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef></dsp:style>
      <dsp:txBody><a:bodyPr anchor="ctr"/><a:lstStyle/><a:p><a:pPr algn="ctr"/><a:r><a:rPr sz="2400"/><a:t>Plan</a:t></a:r></a:p></dsp:txBody>
      <dsp:txXfrm><a:off x="0" y="0"/><a:ext cx="2000000" cy="1000000"/></dsp:txXfrm>
    </dsp:sp>
    <dsp:sp modelId="{C}">
      <dsp:nvSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvSpPr/></dsp:nvSpPr>
      <dsp:spPr>
        <a:xfrm><a:off x="2500000" y="0"/><a:ext cx="2000000" cy="1000000"/></a:xfrm>
        <a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom>
      </dsp:spPr>
      <dsp:txBody><a:bodyPr/><a:p><a:r><a:t>Build</a:t></a:r></a:p></dsp:txBody>
    </dsp:sp>
  </dsp:spTree>
</dsp:drawing>"#;

const CHART: &str = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <c:chart>
    <c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Quarterly </a:t></a:r><a:r><a:t>Revenue</a:t></a:r></a:p></c:rich></c:tx></c:title>
    <c:plotArea>
      <c:layout/>
      <c:barChart>
        <c:barDir val="col"/>
        <c:ser>
          <c:idx val="0"/><c:order val="0"/>
          <c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>Revenue</c:v></c:pt></c:strCache></c:strRef></c:tx>
          <c:cat><c:strRef><c:f>Sheet1!$A$2:$A$4</c:f><c:strCache><c:ptCount val="3"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt><c:pt idx="2"><c:v>Q3</c:v></c:pt></c:strCache></c:strRef></c:cat>
          <c:val><c:numRef><c:f>Sheet1!$B$2:$B$4</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="3"/><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="2"><c:v>12.5</c:v></c:pt></c:numCache></c:numRef></c:val>
        </c:ser>
        <c:ser>
          <c:idx val="1"/><c:order val="1"/>
          <c:tx><c:v>Costs</c:v></c:tx>
          <c:cat><c:numRef><c:numCache><c:pt idx="0"><c:v>2024</c:v></c:pt><c:pt idx="1"><c:v>2025</c:v></c:pt></c:numCache></c:numRef></c:cat>
          <c:val><c:numLit><c:pt idx="0"><c:v>4</c:v></c:pt><c:pt idx="1"><c:v>5</c:v></c:pt></c:numLit></c:val>
        </c:ser>
      </c:barChart>
    </c:plotArea>
  </c:chart>
</c:chartSpace>"#;

fn deck() -> Deck {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", SLIDE),
            ("ppt/slides/_rels/slide1.xml.rels", SLIDE_RELS),
            ("ppt/diagrams/data1.xml", DIAGRAM_DATA),
            ("ppt/diagrams/drawing1.xml", DIAGRAM_DRAWING),
            ("ppt/charts/chart1.xml", CHART),
            ("ppt/media/image1.png", "PNG"),
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
fn smartart_reads_as_a_group_of_the_cached_drawing_shapes_inside_its_frame() {
    let deck = deck();
    let ShapeItem::Group(group) = &deck.slides[0].content.shapes[0] else {
        panic!(
            "SmartArt reads as a group: {:?}",
            deck.slides[0].content.shapes[0]
        );
    };
    assert_eq!(group.name, "Diagram 1");
    assert_eq!(group.id, Some(2));
    assert_eq!(group.diagram_data_rel_id.as_deref(), Some("rId5"));
    // The frame's xfrm is the group's outer transform and its child space
    // starts at the frame's origin, so drawing offsets map straight in.
    let transform = group.transform.expect("frame transform");
    assert_eq!(transform.outer.offset_emu, (914_400, 914_400));
    assert_eq!(transform.outer.extent_emu, (4_572_000, 2_743_200));
    assert_eq!(transform.child_offset_emu, (0, 0));
    assert_eq!(transform.child_extent_emu, (4_572_000, 2_743_200));

    assert_eq!(group.children.len(), 2);
    let ShapeItem::Shape(plan) = &group.children[0] else {
        panic!("shape");
    };
    assert_eq!(plan.preset_geometry.as_deref(), Some("roundRect"));
    assert_eq!(
        plan.transform.expect("xfrm").extent_emu,
        (2_000_000, 1_000_000)
    );
    assert_eq!(
        plan.fill,
        Some(Fill::Solid(Color::Scheme("accent1".into())))
    );
    assert!(plan.style.is_some(), "dsp:style reads like p:style");
    let text = plan.text.as_ref().expect("dsp:txBody");
    assert_eq!(text.paragraphs[0].runs[0].text, "Plan");
    let ShapeItem::Shape(build) = &group.children[1] else {
        panic!("shape");
    };
    assert_eq!(build.transform.expect("xfrm").offset_emu, (2_500_000, 0));
}

#[test]
fn a_chart_reads_its_kind_title_and_series_with_their_cached_points() {
    let deck = deck();
    let ShapeItem::Chart(chart) = &deck.slides[0].content.shapes[1] else {
        panic!("chart: {:?}", deck.slides[0].content.shapes[1]);
    };
    assert_eq!(chart.name, "Chart 2");
    assert_eq!(chart.id, Some(3));
    assert_eq!(chart.chart_rel_id.as_deref(), Some("rId6"));
    assert_eq!(chart.chart_part.as_deref(), Some("ppt/charts/chart1.xml"));
    assert_eq!(
        chart.transform.expect("xfrm").offset_emu,
        (6_096_000, 914_400)
    );
    assert_eq!(chart.kind.as_deref(), Some("barChart"));
    assert_eq!(chart.title.as_deref(), Some("Quarterly Revenue"));

    assert_eq!(chart.series.len(), 2);
    let revenue = &chart.series[0];
    assert_eq!(revenue.name.as_deref(), Some("Revenue"));
    assert_eq!(revenue.categories, vec!["Q1", "Q2", "Q3"]);
    // A missing point (idx 1) stays a hole rather than shifting the rest.
    assert_eq!(revenue.values, vec![Some(10.0), None, Some(12.5)]);

    let costs = &chart.series[1];
    assert_eq!(costs.name.as_deref(), Some("Costs"));
    assert_eq!(costs.categories, vec!["2024", "2025"]);
    assert_eq!(costs.values, vec![Some(4.0), Some(5.0)]);
}

#[test]
fn a_shape_filled_with_a_picture_resolves_its_image_part() {
    let deck = deck();
    let ShapeItem::Shape(photo) = &deck.slides[0].content.shapes[2] else {
        panic!("shape");
    };
    assert_eq!(photo.fill, None);
    assert_eq!(photo.image_rel_id.as_deref(), Some("rId7"));
    assert_eq!(photo.image_part.as_deref(), Some("ppt/media/image1.png"));
}

#[test]
fn a_pattern_fill_keeps_its_preset_and_both_colours() {
    let deck = deck();
    let ShapeItem::Shape(hatched) = &deck.slides[0].content.shapes[3] else {
        panic!("shape");
    };
    assert_eq!(
        hatched.fill,
        Some(Fill::Pattern {
            preset: "dkUpDiag".into(),
            foreground: Color::Srgb("000000".into()),
            background: Color::Srgb("FFFFFF".into()),
        })
    );
}
