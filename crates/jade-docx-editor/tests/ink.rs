//! Word ink as the 描画 tab writes it: an InkML part behind a
//! `w14:contentPart`, in an `mc:AlternateContent` with a picture fallback —
//! read back into strokes, kept, replaced and removed, and undone.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::commands::set_ink_drawings::SetInkDrawingsCommand;
use jade_docx_editor::commands::{CommandContext, CommandExecutor};
use jade_docx_editor::word_document::ink_drawing::InkDrawingSpec;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_reader::inkml::{InkBrush, InkTool, InkTrace};

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

fn hello() -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    load_bytes(&std::fs::read(path).expect("fixture"))
}

fn part(bytes: &[u8], name: &str) -> Option<String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut xml = String::new();
    archive.by_name(name).ok()?.read_to_string(&mut xml).ok()?;
    Some(xml)
}

fn names(bytes: &[u8]) -> Vec<String> {
    let archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    archive
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect()
}

fn stroke(points: &[(f64, f64)], tool: InkTool, color: &str) -> InkTrace {
    InkTrace {
        points: points.to_vec(),
        brush: InkBrush {
            tool,
            color: color.into(),
            width_px: 3.78,
            height_px: if tool == InkTool::Highlighter {
                22.68
            } else {
                3.78
            },
            ..Default::default()
        },
    }
}

fn apply(document: &mut WordDocument, executor: &mut CommandExecutor, specs: Vec<InkDrawingSpec>) {
    let mut selection = None;
    let mut context = CommandContext::new(document, &mut selection);
    executor
        .execute(Box::new(SetInkDrawingsCommand::new(specs)), &mut context)
        .expect("set ink");
}

#[test]
fn ink_is_written_as_word_ink_and_read_back() {
    let mut document = hello();
    let mut executor = CommandExecutor::new();
    let traces = vec![
        stroke(
            &[(100.0, 120.0), (160.0, 150.0), (220.0, 130.0)],
            InkTool::Pen,
            "#E71224",
        ),
        stroke(
            &[(100.0, 200.0), (300.0, 200.0)],
            InkTool::Highlighter,
            "#FFFF00",
        ),
    ];
    apply(
        &mut document,
        &mut executor,
        vec![InkDrawingSpec::New {
            paragraph_index: 0,
            traces: traces.clone(),
        }],
    );
    let drawings = document.ink_drawings();
    assert_eq!(drawings.len(), 1);
    let drawing = &drawings[0];
    assert!(drawing.page_relative);
    // The box holds the strokes with their brushes.
    assert!(
        (drawing.x_px - (100.0 - 11.34)).abs() < 0.01,
        "{}",
        drawing.x_px
    );
    assert!(
        (drawing.y_px - (120.0 - 1.89)).abs() < 0.01,
        "{}",
        drawing.y_px
    );

    let bytes = document.export_docx().expect("export");
    let xml = part(&bytes, "word/document.xml").unwrap();
    assert!(xml.contains("<mc:AlternateContent>"), "{xml}");
    assert!(xml.contains(r#"<mc:Choice Requires="wpi">"#));
    assert!(xml.contains("<w14:contentPart"));
    assert!(xml.contains("<mc:Fallback>"));
    assert!(xml.contains("<pic:pic"));
    assert!(xml.contains("xmlns:wpi="), "the ink namespace is declared");
    let ink = part(&bytes, "word/ink/ink1.xml").expect("the ink part");
    assert!(ink.contains("<inkml:trace"));
    assert!(ink.contains(r#"name="tip" value="rectangle""#));
    let content_types = part(&bytes, "[Content_Types].xml").unwrap();
    assert!(content_types.contains("application/inkml+xml"));
    let rels = part(&bytes, "word/_rels/document.xml.rels").unwrap();
    assert!(rels.contains("ink/ink1.xml"));
    assert!(names(&bytes)
        .iter()
        .any(|name| name.starts_with("word/media/ink")));

    // Opened again, the strokes are there.
    let reopened = load_bytes(&bytes);
    let read = reopened.ink_drawings();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].traces.len(), 2);
    assert_eq!(read[0].traces[0].brush.color, "#E71224");
    assert_eq!(read[0].traces[1].brush.tool, InkTool::Highlighter);
    let original = &drawing.traces[0].points;
    for (a, b) in read[0].traces[0].points.iter().zip(original) {
        assert!(
            (a.0 - b.0).abs() < 0.1 && (a.1 - b.1).abs() < 0.1,
            "{a:?} {b:?}"
        );
    }
}

#[test]
fn ink_is_kept_replaced_removed_and_undone() {
    let mut document = hello();
    let mut executor = CommandExecutor::new();
    let first = stroke(&[(10.0, 10.0), (40.0, 40.0)], InkTool::Pen, "#000000");
    let second = stroke(&[(50.0, 300.0), (80.0, 330.0)], InkTool::Pencil, "#0070C0");
    apply(
        &mut document,
        &mut executor,
        vec![
            InkDrawingSpec::New {
                paragraph_index: 0,
                traces: vec![first],
            },
            InkDrawingSpec::New {
                paragraph_index: 0,
                traces: vec![second],
            },
        ],
    );
    let drawings = document.ink_drawings();
    assert_eq!(drawings.len(), 2);
    // Keep the second; the first goes, with its parts.
    apply(
        &mut document,
        &mut executor,
        vec![InkDrawingSpec::Keep {
            paragraph_index: drawings[1].paragraph_index,
            run_index: drawings[1].run_index,
        }],
    );
    let left = document.ink_drawings();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].traces[0].brush.tool, InkTool::Pencil);
    let bytes = document.export_docx().expect("export");
    let ink_parts = names(&bytes)
        .into_iter()
        .filter(|name| name.starts_with("word/ink/"))
        .count();
    assert_eq!(ink_parts, 1);
    let rels = part(&bytes, "word/_rels/document.xml.rels").unwrap();
    assert_eq!(rels.matches("ink/ink").count(), 1);

    let mut selection = None;
    {
        let mut context = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut context).expect("undo");
    }
    assert_eq!(document.ink_drawings().len(), 2);
}

/// `hello.docx` with `document.xml`, the document's relationships and the
/// content types passed through `edit`, and `extra` parts added.
fn hello_with(edit: impl Fn(&str, String) -> String, extra: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let bytes = std::fs::read(path).expect("fixture");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("entry");
        let name = file.name().unwrap().into_owned();
        let mut content = Vec::new();
        file.read_to_end(&mut content).expect("read");
        let content = match String::from_utf8(content) {
            Ok(text) => edit(&name, text).into_bytes(),
            Err(error) => error.into_bytes(),
        };
        out.start_file(name, options).expect("start");
        out.write_all(&content).expect("write");
    }
    for (name, content) in extra {
        out.start_file(*name, options).expect("start");
        out.write_all(content.as_bytes()).expect("write");
    }
    out.finish().expect("finish").into_inner()
}

/// Ink as 挿入 › 図形's pen wrote it before the 描画 tab: a floating drawing
/// whose graphic is a bare `w14:contentPart`, with no picture fallback.
#[test]
fn ink_without_a_fallback_picture_reads_back_and_keeps_its_part() {
    let inkml = r##"<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions><inkml:context xml:id="ctx0"><inkml:inkSource xml:id="inkSrc0"><inkml:traceFormat><inkml:channel name="X" type="integer" max="32767" units="cm"/><inkml:channel name="Y" type="integer" max="32767" units="cm"/></inkml:traceFormat><inkml:channelProperties><inkml:channelProperty channel="X" name="resolution" value="1000" units="1/cm"/><inkml:channelProperty channel="Y" name="resolution" value="1000" units="1/cm"/></inkml:channelProperties></inkml:inkSource></inkml:context><inkml:brush xml:id="br0"><inkml:brushProperty name="width" value="0.05" units="cm"/><inkml:brushProperty name="height" value="0.05" units="cm"/><inkml:brushProperty name="color" value="#E71224"/></inkml:brush></inkml:definitions><inkml:trace contextRef="#ctx0" brushRef="#br0">0 0, 1000 500, 2000 0</inkml:trace></inkml:ink>"##;
    let drawing = r#"<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" relativeHeight="251665408" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>914400</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>914400</wp:posOffset></wp:positionV><wp:extent cx="762000" cy="190500"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="7" name="インク 7"/><wp:cNvGraphicFramePr/><w14:contentPart xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" bwMode="auto" r:id="rIdInk7"><w14:nvContentPartPr><w14:cNvContentPartPr/></w14:nvContentPartPr><w14:xfrm><a:off xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" x="0" y="0"/><a:ext xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" cx="762000" cy="190500"/></w14:xfrm></w14:contentPart></wp:anchor></w:drawing></w:r>"#;
    let bytes = hello_with(
        |name, text| {
            match name {
            "word/document.xml" => {
                let at = text.find("</w:p>").expect("a paragraph");
                let mut text = text;
                text.insert_str(at, drawing);
                if !text.contains("xmlns:wp=") {
                    text = text.replacen(
                        "<w:document ",
                        r#"<w:document xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
                        1,
                    );
                }
                if !text.contains("xmlns:r=") {
                    text = text.replacen(
                        "<w:document ",
                        r#"<w:document xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
                        1,
                    );
                }
                text
            }
            "word/_rels/document.xml.rels" => text.replace(
                "</Relationships>",
                r#"<Relationship Id="rIdInk7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="ink/ink7.xml"/></Relationships>"#,
            ),
            "[Content_Types].xml" => text.replace(
                "</Types>",
                r#"<Override PartName="/word/ink/ink7.xml" ContentType="application/inkml+xml"/></Types>"#,
            ),
            _ => text,
        }
        },
        &[("word/ink/ink7.xml", inkml)],
    );
    let document = load_bytes(&bytes);
    let drawings = document.ink_drawings();
    assert_eq!(drawings.len(), 1, "the ink is read as ink");
    assert_eq!(drawings[0].traces.len(), 1);
    assert_eq!(drawings[0].traces[0].points.len(), 3);
    assert_eq!(drawings[0].traces[0].brush.color, "#E71224");

    // Saved untouched, the drawing and its part survive.
    let saved = document.export_docx().expect("export");
    assert!(part(&saved, "word/document.xml")
        .unwrap()
        .contains("contentPart"));
    assert!(part(&saved, "word/ink/ink7.xml").is_some());
    assert_eq!(load_bytes(&saved).ink_drawings().len(), 1);
}
