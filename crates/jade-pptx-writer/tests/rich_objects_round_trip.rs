//! A deck with the objects business decks carry — a table, a chart, a
//! SmartArt diagram and a video — keeps every one of them when a shape on
//! the same slide is typed into and the deck is saved: the slide still holds
//! each object, the parts they point at come back byte for byte, and the
//! relationships and content types that tie them in are still there.
//!
//! The corpus has no such deck, so this one is built from `sample-deck.pptx`
//! with the objects added to its first slide as PowerPoint writes them. The
//! slide also draws them — the table's cells, the chart's categories, the
//! diagram's boxes — and draws the same after the save.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;

use jade_pptx_writer::{shape_text, EditSession};

const TABLE: &str = r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="9001" name="Table 1"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="838200" y="2000000"/><a:ext cx="4000000" cy="740000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1" bandRow="1"/><a:tblGrid><a:gridCol w="2000000"/><a:gridCol w="2000000"/></a:tblGrid><a:tr h="370000"><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>品目</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>金額</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr><a:tr h="370000"><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>表セル</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>1,200</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#;

const CHART: &str = r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="9002" name="Chart 2"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="5000000" y="2000000"/><a:ext cx="3000000" cy="2000000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rIdChart1"/></a:graphicData></a:graphic></p:graphicFrame>"#;

const SMART_ART: &str = r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="9003" name="Diagram 3"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="838200" y="3000000"/><a:ext cx="4000000" cy="1500000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" r:dm="rIdDgmData1" r:lo="rIdDgmLayout1" r:qs="rIdDgmStyle1" r:cs="rIdDgmColors1"/></a:graphicData></a:graphic></p:graphicFrame>"#;

const VIDEO: &str = r#"<p:pic><p:nvPicPr><p:cNvPr id="9004" name="Video 4"><a:hlinkClick r:id="" action="ppaction://media"/></p:cNvPr><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr><a:videoFile r:link="rIdVideo1"/><p:extLst><p:ext uri="{DAA4B4D4-6D71-4841-9C94-3DE7FCFB9230}"><p14:media xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" r:embed="rIdMedia1"/></p:ext></p:extLst></p:nvPr></p:nvPicPr><p:blipFill><a:blip r:embed="rIdPoster1"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="5000000" y="4200000"/><a:ext cx="1600000" cy="900000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#;

const RELATIONSHIPS: &str = r#"<Relationship Id="rIdChart1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/><Relationship Id="rIdDgmData1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData" Target="../diagrams/data1.xml"/><Relationship Id="rIdDgmLayout1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramLayout" Target="../diagrams/layout1.xml"/><Relationship Id="rIdDgmStyle1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramQuickStyle" Target="../diagrams/quickStyle1.xml"/><Relationship Id="rIdDgmColors1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramColors" Target="../diagrams/colors1.xml"/><Relationship Id="rIdDgmDrawing1" Type="http://schemas.microsoft.com/office/2007/relationships/diagramDrawing" Target="../diagrams/drawing1.xml"/><Relationship Id="rIdVideo1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/video" Target="../media/media1.mp4"/><Relationship Id="rIdMedia1" Type="http://schemas.microsoft.com/office/2007/relationships/media" Target="../media/media1.mp4"/><Relationship Id="rIdPoster1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/poster1.png"/>"#;

const CONTENT_TYPES: &str = r#"<Default Extension="mp4" ContentType="video/mp4"/><Override PartName="/ppt/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/><Override PartName="/ppt/diagrams/data1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml"/><Override PartName="/ppt/diagrams/layout1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.diagramLayout+xml"/><Override PartName="/ppt/diagrams/quickStyle1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.diagramStyle+xml"/><Override PartName="/ppt/diagrams/colors1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.diagramColors+xml"/><Override PartName="/ppt/diagrams/drawing1.xml" ContentType="application/vnd.ms-office.drawingml.diagramDrawing+xml"/>"#;

/// The parts the objects point at, as PowerPoint would store them (trimmed
/// to what identifies them), and a few bytes standing in for the media.
fn object_parts() -> Vec<(&'static str, Vec<u8>)> {
    let xml = |body: &str| {
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n{body}")
            .into_bytes()
    };
    vec![
        (
            "ppt/charts/chart1.xml",
            xml(
                r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:ser><c:idx val="0"/><c:order val="0"/><c:cat><c:strLit><c:ptCount val="2"/><c:pt idx="0"><c:v>上期</c:v></c:pt><c:pt idx="1"><c:v>下期</c:v></c:pt></c:strLit></c:cat><c:val><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>120</c:v></c:pt><c:pt idx="1"><c:v>180</c:v></c:pt></c:numLit></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="b"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="l"/><c:crossAx val="1"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#,
            ),
        ),
        (
            "ppt/diagrams/data1.xml",
            xml(
                r#"<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dgm:ptLst><dgm:pt modelId="{0}" type="doc"/><dgm:pt modelId="{1}"><dgm:t><a:bodyPr/><a:p><a:r><a:t>企画</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="{2}"><dgm:t><a:bodyPr/><a:p><a:r><a:t>実行</a:t></a:r></a:p></dgm:t></dgm:pt></dgm:ptLst><dgm:cxnLst><dgm:cxn modelId="{3}" srcId="{0}" destId="{1}" srcOrd="0" destOrd="0"/><dgm:cxn modelId="{4}" srcId="{0}" destId="{2}" srcOrd="1" destOrd="0"/></dgm:cxnLst></dgm:dataModel>"#,
            ),
        ),
        (
            "ppt/diagrams/layout1.xml",
            xml(
                r#"<dgm:layoutDef xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" uniqueId="urn:microsoft.com/office/officeart/2005/8/layout/process1"><dgm:layoutNode name="Name0"/></dgm:layoutDef>"#,
            ),
        ),
        (
            "ppt/diagrams/quickStyle1.xml",
            xml(
                r#"<dgm:styleDef xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" uniqueId="urn:microsoft.com/office/officeart/2005/8/quickstyle/simple1"/>"#,
            ),
        ),
        (
            "ppt/diagrams/colors1.xml",
            xml(
                r#"<dgm:colorsDef xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" uniqueId="urn:microsoft.com/office/officeart/2005/8/colors/accent1_2"/>"#,
            ),
        ),
        (
            "ppt/diagrams/drawing1.xml",
            xml(
                r#"<dsp:drawing xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dsp:spTree><dsp:nvGrpSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvGrpSpPr/></dsp:nvGrpSpPr><dsp:grpSpPr/><dsp:sp modelId="{1}"><dsp:nvSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvSpPr/></dsp:nvSpPr><dsp:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1800000" cy="1000000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></dsp:spPr><dsp:txBody><a:bodyPr/><a:p><a:r><a:t>企画</a:t></a:r></a:p></dsp:txBody></dsp:sp><dsp:sp modelId="{2}"><dsp:nvSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvSpPr/></dsp:nvSpPr><dsp:spPr><a:xfrm><a:off x="2200000" y="0"/><a:ext cx="1800000" cy="1000000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></dsp:spPr><dsp:txBody><a:bodyPr/><a:p><a:r><a:t>実行</a:t></a:r></a:p></dsp:txBody></dsp:sp></dsp:spTree></dsp:drawing>"#,
            ),
        ),
        (
            "ppt/media/media1.mp4",
            b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00mp42isom jade video".to_vec(),
        ),
        (
            "ppt/media/poster1.png",
            b"\x89PNG\r\n\x1a\n jade poster".to_vec(),
        ),
    ]
}

fn parts(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    (0..archive.len())
        .map(|index| {
            let mut file = archive.by_index(index).expect("entry");
            let mut data = Vec::new();
            file.read_to_end(&mut data).expect("read");
            (file.name().to_string(), data)
        })
        .collect()
}

fn text(parts: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(parts[name].clone()).expect("utf-8")
}

/// sample-deck.pptx with the four objects on its first slide.
fn rich_deck() -> Vec<u8> {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/sample-deck.pptx");
    let mut parts = parts(&std::fs::read(fixture).expect("fixture"));
    let slide = text(&parts, "ppt/slides/slide1.xml").replacen(
        "</p:spTree>",
        &format!("{TABLE}{CHART}{SMART_ART}{VIDEO}</p:spTree>"),
        1,
    );
    parts.insert("ppt/slides/slide1.xml".into(), slide.into_bytes());
    let rels = text(&parts, "ppt/slides/_rels/slide1.xml.rels").replacen(
        "</Relationships>",
        &format!("{RELATIONSHIPS}</Relationships>"),
        1,
    );
    parts.insert("ppt/slides/_rels/slide1.xml.rels".into(), rels.into_bytes());
    let types = text(&parts, "[Content_Types].xml").replacen(
        "</Types>",
        &format!("{CONTENT_TYPES}</Types>"),
        1,
    );
    parts.insert("[Content_Types].xml".into(), types.into_bytes());
    for (name, data) in object_parts() {
        parts.insert(name.into(), data);
    }

    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        let options = zip::write::SimpleFileOptions::default();
        // The content types first, as a package lists them.
        let mut names: Vec<_> = parts.keys().cloned().collect();
        names.sort_by_key(|name| name != "[Content_Types].xml");
        for name in names {
            writer.start_file(name.as_str(), options).expect("start");
            writer.write_all(&parts[&name]).expect("write");
        }
        writer.finish().expect("finish");
    }
    buffer
}

#[test]
fn typing_beside_a_table_chart_smartart_and_video_keeps_them_all() {
    let original = rich_deck();
    let mut session = EditSession::open(&original).expect("the deck opens");
    // The title, on the slide that holds the objects.
    assert_eq!(
        shape_text(&original, 0, 2).expect("title").text,
        "Jade Slides"
    );
    session
        .set_shape_text(0, 2, "入力した", false)
        .expect("typing");
    session.mark_saved();
    let saved = session.bytes().to_vec();
    assert_eq!(shape_text(&saved, 0, 2).expect("title").text, "入力した");
    EditSession::open(&saved).expect("the saved deck opens");

    let (before, after) = (parts(&original), parts(&saved));
    // Every part the objects point at, byte for byte.
    for (name, data) in object_parts() {
        assert_eq!(
            after.get(name),
            Some(&data),
            "{name} changed or was dropped"
        );
    }
    // The slide still holds each object and what identifies it.
    let slide = text(&after, "ppt/slides/slide1.xml");
    for marker in [
        "表セル",
        "1,200",
        r#"<a:gridCol w="2000000"/>"#,
        r#"r:id="rIdChart1""#,
        r#"r:dm="rIdDgmData1""#,
        r#"r:cs="rIdDgmColors1""#,
        r#"<a:videoFile r:link="rIdVideo1"/>"#,
        r#"r:embed="rIdMedia1""#,
        r#"r:embed="rIdPoster1""#,
    ] {
        assert!(slide.contains(marker), "the slide lost {marker}:\n{slide}");
    }
    // And the relationships and content types that tie them in.
    let rels = text(&after, "ppt/slides/_rels/slide1.xml.rels");
    for id in [
        "rIdChart1",
        "rIdDgmData1",
        "rIdDgmLayout1",
        "rIdDgmStyle1",
        "rIdDgmColors1",
        "rIdDgmDrawing1",
        "rIdVideo1",
        "rIdMedia1",
        "rIdPoster1",
    ] {
        assert!(
            rels.contains(&format!(r#"Id="{id}""#)),
            "the slide's relationships lost {id}"
        );
    }
    let types = text(&after, "[Content_Types].xml");
    for part in [
        "/ppt/charts/chart1.xml",
        "/ppt/diagrams/data1.xml",
        "/ppt/diagrams/drawing1.xml",
    ] {
        assert!(types.contains(part), "the content types lost {part}");
    }
    assert!(
        types.contains(r#"Extension="mp4""#),
        "the content types lost mp4"
    );
    // Nothing else in the package changed but the slide typed into.
    for (name, data) in &before {
        if name != "ppt/slides/slide1.xml" {
            assert_eq!(after.get(name), Some(data), "{name} changed");
        }
    }
}

/// What drawing slide `index` of `bytes` paints.
fn painted(bytes: &[u8], index: usize) -> Vec<jade_painter::recording::Op> {
    let deck = jade_pptx_reader::read_deck(bytes).expect("the deck reads");
    let painter = jade_painter::recording::RecordingPainter::new();
    jade_pptx_render::render_slide(
        &painter,
        &jade_pptx_layout::layout_deck_slide(&deck, index, &jade_pptx_layout::HeuristicMeasure)
            .expect("the slide lays out"),
        &Default::default(),
    )
    .expect("the slide renders");
    painter.ops()
}

fn texts(ops: &[jade_painter::recording::Op]) -> Vec<String> {
    ops.iter()
        .filter_map(|op| match op {
            jade_painter::recording::Op::FillText(text, _, _) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_slide_with_those_objects_draws_and_draws_the_same_after_a_save() {
    let original = rich_deck();
    let before = painted(&original, 0);
    let drawn = texts(&before).concat();
    // The table's cells, the chart's categories, the diagram's boxes.
    for text in [
        "品目",
        "金額",
        "表セル",
        "1,200",
        "上期",
        "下期",
        "企画",
        "実行",
    ] {
        assert!(drawn.contains(text), "{text} is not drawn: {drawn}");
    }

    let mut session = EditSession::open(&original).expect("the deck opens");
    session
        .set_shape_text(0, 2, "入力した", false)
        .expect("typing");
    let after = painted(session.bytes(), 0);
    // The title is the only text that changed; everything else paints as it did.
    let swap = |text: String| {
        if text == "Jade Slides" {
            "入力した".to_string()
        } else {
            text
        }
    };
    assert_eq!(
        texts(&before).into_iter().map(swap).collect::<Vec<_>>(),
        texts(&after)
    );
    assert_eq!(before.len(), after.len());
}
