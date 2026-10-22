//! Word ink in a file Word saved — an InkML part behind a
//! `w14:contentPart`, coordinates in differences — is read into strokes and
//! drawn as strokes, and survives being saved again.

use super::recording_host::RecordingHost;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::{Op, RecordingPainter};
use std::io::Write;

const INK: &str = r##"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions><inkml:context xml:id="ctx0"><inkml:inkSource xml:id="inkSrc0"><inkml:traceFormat><inkml:channel name="X" type="integer" max="32767" units="cm"/><inkml:channel name="Y" type="integer" max="32767" units="cm"/><inkml:channel name="T" type="integer" max="2.14748E9" units="dev"/></inkml:traceFormat><inkml:channelProperties><inkml:channelProperty channel="X" name="resolution" value="1000" units="1/cm"/><inkml:channelProperty channel="Y" name="resolution" value="1000" units="1/cm"/><inkml:channelProperty channel="T" name="resolution" value="1" units="1/dev"/></inkml:channelProperties></inkml:inkSource><inkml:timestamp xml:id="ts0" timeString="2026-10-01T00:00:00Z"/></inkml:context><inkml:brush xml:id="br0"><inkml:brushProperty name="width" value="0.05" units="cm"/><inkml:brushProperty name="height" value="0.05" units="cm"/><inkml:brushProperty name="color" value="#E71224"/></inkml:brush></inkml:definitions><inkml:trace contextRef="#ctx0" brushRef="#br0">5000 5000 0,'500'0'10,'500'250'10,"0"0'10</inkml:trace></inkml:ink>"##;

fn word_ink_docx() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/ink/ink1.xml" ContentType="application/inkml+xml"/></Types>"#;
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="ink/ink1.xml"/></Relationships>"#;
    let document = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:wpi="http://schemas.microsoft.com/office/word/2010/wordprocessingInk" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="w14 wpi"><w:body><w:p><w:r><w:t>Ink below.</w:t></w:r><w:r><mc:AlternateContent><mc:Choice Requires="wpi"><w:drawing><wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" relativeHeight="251659264" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>1905000</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>1905000</wp:posOffset></wp:positionV><wp:extent cx="381000" cy="114300"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="1" name="Ink 1"/><wp:cNvGraphicFramePr/><w14:contentPart bwMode="auto" r:id="rId4"><w14:nvContentPartPr><w14:cNvContentPartPr/></w14:nvContentPartPr><w14:xfrm><a:off x="0" y="0"/><a:ext cx="381000" cy="114300"/></w14:xfrm></w14:contentPart></wp:anchor></w:drawing></mc:Choice></mc:AlternateContent></w:r></w:p><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#;
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (path, content) in [
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", root_rels),
            ("word/document.xml", document),
            ("word/_rels/document.xml.rels", doc_rels),
            ("word/ink/ink1.xml", INK),
        ] {
            zip.start_file(path, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

fn render(model: &WordDocument) -> Vec<Op> {
    let painter = RecordingPainter::new();
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        false,
    )
    .expect("recording renderer");
    renderer
        .render(
            model.document.as_ref().expect("document"),
            &Options::default(),
        )
        .expect("render");
    painter.ops()
}

#[test]
fn word_ink_is_read_and_drawn_as_strokes() {
    let model =
        futures::executor::block_on(WordDocument::load(&word_ink_docx(), &Options::default()))
            .expect("load");
    let drawings = model.ink_drawings();
    assert_eq!(drawings.len(), 1);
    let ink = &drawings[0];
    assert!(ink.page_relative);
    assert_eq!((ink.x_px, ink.y_px), (200.0, 200.0));
    assert_eq!(ink.traces[0].points.len(), 4);
    assert_eq!(ink.traces[0].brush.color, "#E71224");

    let ops = render(&model);
    let red = ops
        .iter()
        .position(|op| matches!(op, Op::SetStrokeStyle(color) if color == "#E71224"))
        .expect("the stroke's colour");
    assert!(ops[red..].iter().any(|op| matches!(op, Op::Stroke)));
    // The stroke runs across the box from its left edge.
    let first_move = ops[red..]
        .iter()
        .find_map(|op| match op {
            Op::MoveTo(x, y) => Some((*x, *y)),
            _ => None,
        })
        .unwrap();
    assert!(
        first_move.0 >= 200.0 && first_move.0 < 205.0,
        "{first_move:?}"
    );
    assert!(!ops.iter().any(|op| matches!(op, Op::DrawImage { .. })));
}

#[test]
fn word_ink_is_saved_again_as_word_ink() {
    let mut model =
        futures::executor::block_on(WordDocument::load(&word_ink_docx(), &Options::default()))
            .expect("load");
    // An edit to the paragraph holding the ink rebuilds its runs.
    model.paragraph_mut(0).unwrap().runs[0].text = "Ink, below.".into();
    model.sync_paragraph_text(0);
    let bytes = model.export_docx().expect("export");
    let reopened =
        futures::executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
    let drawings = reopened.ink_drawings();
    assert_eq!(drawings.len(), 1);
    assert_eq!(drawings[0].traces[0].points.len(), 4);
}

#[test]
fn a_resized_ink_drawing_scales_its_strokes() {
    let mut model =
        futures::executor::block_on(WordDocument::load(&word_ink_docx(), &Options::default()))
            .expect("load");
    // How far right the red stroke reaches from the drawing's left edge.
    let extent = |ops: &[Op]| {
        let start = ops
            .iter()
            .position(|op| matches!(op, Op::SetStrokeStyle(color) if color == "#E71224"))
            .expect("the stroke");
        ops[start..]
            .iter()
            .take_while(|op| !matches!(op, Op::Stroke))
            .filter_map(|op| match op {
                Op::MoveTo(x, _) | Op::LineTo(x, _) => Some(*x),
                _ => None,
            })
            .fold(f64::MIN, f64::max)
            - 200.0
    };
    let before = extent(&render(&model));
    let image = model.paragraph_mut(0).unwrap().runs[1]
        .image
        .as_mut()
        .expect("the ink drawing");
    image.width_px = image.width_px.map(|width| width * 2.0);
    let after = extent(&render(&model));
    assert!((after - 2.0 * before).abs() < 2.0, "{before} -> {after}");
}
