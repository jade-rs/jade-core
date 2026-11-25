//! NUMPAGES totals settled for one document are not shown in the next one a
//! reused renderer lays out.
use super::recording_host::RecordingHost;
use super::*;
use jade_painter::recording::{Op, RecordingPainter};
use std::io::Write;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// `pages` pages with the footer `PAGE of NUMPAGES`.
fn paged_docx(pages: usize) -> Vec<u8> {
    let field = |code: &str| {
        format!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {code} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
        )
    };
    let footer = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr xmlns:w="{W}" xmlns:r="{R}"><w:p>{}<w:r><w:t xml:space="preserve"> of </w:t></w:r>{}</w:p></w:ftr>"#,
        field("PAGE"),
        field("NUMPAGES"),
    );
    let body: String = (0..pages)
        .map(|page| {
            let break_before = if page == 0 {
                ""
            } else {
                "<w:pPr><w:pageBreakBefore/></w:pPr>"
            };
            format!("<w:p>{break_before}<w:r><w:t>Body {page}</w:t></w:r></w:p>")
        })
        .collect();
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}<w:sectPr><w:footerReference w:type="default" r:id="rIdFooter"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let mut bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#.to_string(),
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
            ),
            (
                "word/_rels/document.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdFooter" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/></Relationships>"#.to_string(),
            ),
            ("word/document.xml", document),
            ("word/footer1.xml", footer),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    bytes
}

fn load(pages: usize) -> crate::word_document::WordDocument {
    futures::executor::block_on(crate::word_document::WordDocument::load(
        &paged_docx(pages),
        &Options::default(),
    ))
    .unwrap()
}

/// The text the pass drew, one string per `fill_text`.
fn render(
    renderer: &mut CanvasRenderer<RecordingPainter>,
    painter: &RecordingPainter,
    document: &crate::word_document::WordDocument,
) -> Vec<String> {
    let before = painter.ops().len();
    renderer.sync_word_document(document);
    renderer
        .render_with_numbering(
            document.document.as_ref().unwrap(),
            &Options::default(),
            document.numbering_part.as_ref(),
            document.header_lines.as_deref(),
            document.footer_lines.as_deref(),
            Some(&document.comments),
        )
        .unwrap();
    painter.ops()[before..]
        .iter()
        .filter_map(|op| match op {
            Op::FillText(text, ..) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Whether a footer drew `of {total}` (the recording draws a glyph at a
/// time, with formatting marks after each paragraph).
fn shows_total(texts: &[String], total: usize) -> bool {
    texts.concat().contains(&format!(" of {total}\u{b7}"))
}

#[test]
fn a_reused_renderer_forgets_the_previous_documents_page_count() {
    let long = load(7);
    let short = load(2);
    for reset in [false, true] {
        let painter = RecordingPainter::new();
        let mut renderer = CanvasRenderer::with_host(
            Box::new(RecordingHost {
                painter: painter.clone(),
            }),
            false,
        )
        .unwrap();
        render(&mut renderer, &painter, &long);
        let settled = render(&mut renderer, &painter, &long);
        assert!(shows_total(&settled, 7), "{settled:?}");
        if reset {
            // What loading another document does.
            renderer.reset_page_totals();
        }
        let next = render(&mut renderer, &painter, &short);
        assert_eq!(shows_total(&next, 7), !reset, "reset {reset}: {next:?}");
        let settled = render(&mut renderer, &painter, &short);
        assert!(shows_total(&settled, 2), "{settled:?}");
    }
}
