//! Section-break edits rendered on a recording painter: an edited document
//! has to lay out as a reload of the file it saves.

use super::recording_host::recording_renderer;
use super::*;
use crate::commands::delete_text::{DeleteDirection, DeleteTextCommand};
use crate::commands::{CommandContext, CommandExecutor};
use crate::word_document::WordDocument;
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use jade_painter::recording::RecordingPainter;
use std::io::Write;

pub(super) fn build_docx_with_body(body_xml: &str) -> Vec<u8> {
    use zip::write::SimpleFileOptions;

    let document_xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body_xml}</w:body></w:document>"#
    );
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default();
        for (path, content) in [
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            ("word/document.xml", document_xml.as_str()),
        ] {
            zip.start_file(path, options).expect("start entry");
            zip.write_all(content.as_bytes()).expect("write entry");
        }
        zip.finish().expect("finish package");
    }
    cursor.into_inner()
}

/// Line count of every paragraph surface after rendering `word_doc`.
fn line_counts(
    renderer: &mut CanvasRenderer<RecordingPainter>,
    word_doc: &WordDocument,
) -> Vec<usize> {
    let document = word_doc.document.as_ref().expect("document");
    renderer
        .render(document, &Options::default())
        .expect("render");
    renderer
        .selection
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.lines.len())
        .collect()
}

/// Backspace at the start of the first landscape paragraph removes the
/// portrait section's break. At 10px a character an 80-character paragraph
/// fits on one landscape A4 line (930px of text width) but not on a portrait
/// one (602px), so the edited document has to wrap it as a reload does.
#[test]
fn joining_the_first_section_wraps_like_a_reload() {
    let margins = r#"<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;
    let paragraph = |text: &str| format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>");
    let long = "abcd ".repeat(16);
    let mut body = String::new();
    for index in 0..3 {
        body.push_str(&paragraph(&format!("Portrait {index}")));
    }
    body.push_str(&format!(
        r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/>{margins}</w:sectPr></w:pPr><w:r><w:t>Portrait 3</w:t></w:r></w:p>"#
    ));
    for _ in 0..3 {
        body.push_str(&paragraph(&long));
    }
    body.push_str(&format!(
        r#"<w:sectPr><w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/>{margins}</w:sectPr>"#
    ));
    let data = build_docx_with_body(&body);

    let mut word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let mut renderer = recording_renderer();
    let before = line_counts(&mut renderer, &word_doc);
    assert_eq!(
        &before[4..],
        &[1, 1, 1],
        "landscape section before the edit"
    );

    let range = SelectionRange::collapsed(TextPosition::new(4, 0));
    let mut executor = CommandExecutor::new();
    {
        let mut selection = Some(range);
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        executor
            .execute(
                Box::new(DeleteTextCommand::new(range, DeleteDirection::Backward)),
                &mut ctx,
            )
            .expect("backspace");
    }
    let edited = line_counts(&mut renderer, &word_doc);

    let exported = word_doc.export_docx().expect("export");
    let reloaded = futures::executor::block_on(WordDocument::load(&exported, &Options::default()))
        .expect("reload");
    let fresh = line_counts(&mut recording_renderer(), &reloaded);
    assert_eq!(&fresh[4..], &[1, 1], "reloaded landscape paragraphs");
    assert_eq!(edited, fresh, "the edited document wraps as its reload");
}
