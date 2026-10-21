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
    archive.file_names().map(str::to_string).collect()
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
