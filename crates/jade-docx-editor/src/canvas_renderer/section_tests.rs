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

mod parity_filler {
    //! The page an oddPage/evenPage break inserts to reach the required
    //! parity. Word prints it blank (no header, footer, page borders or
    //! watermark) yet it takes a page number, as the Word for Mac capture of
    //! `section-break-types.docx` shows (an empty page 3 between the
    //! evenPage break's source and target sections).

    use super::super::continuous_chrome_tests::{package, paragraphs, section, PAGE_FIELD};
    use super::super::recording_host::recording_renderer;
    use super::super::*;
    use crate::word_document::WordDocument;
    use jade_painter::recording::{Op, RecordingPainter};

    const RED: &str = "FF0000";
    const BLUE: &str = "0000FF";
    const PAGE_BREAK: &str = r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#;

    /// Section A's `a_body`, then section B starting with `break_type` and
    /// holding a PAGE field. Both sections have headers and footers
    /// followed by a PAGE field, and page borders.
    fn two_sections(a_body: String, break_type: &str) -> Vec<u8> {
        package(
            &[
                section("A", RED, "nextPage", a_body, ""),
                section("B", BLUE, break_type, PAGE_FIELD.to_string(), ""),
            ],
            "",
        )
    }

    /// Renders `data` with a text watermark and returns the renderer with
    /// the ops of each page, split at its full-page clear.
    fn render(data: &[u8]) -> (CanvasRenderer<RecordingPainter>, Rc<Document>, Vec<Vec<Op>>) {
        let word_doc = futures::executor::block_on(WordDocument::load(data, &Options::default()))
            .expect("load");
        let mut document = Document::clone(word_doc.document.as_deref().expect("document"));
        document.watermark_text = Some("WM".into());
        let document = Rc::new(document);
        let mut renderer = recording_renderer();
        renderer.sync_document_header_footer_stories(&word_doc);
        renderer
            .render(&document, &Options::default())
            .expect("render");
        let pages = split_pages(&renderer.host.painter().inner().ops());
        (renderer, document, pages)
    }

    fn split_pages(ops: &[Op]) -> Vec<Vec<Op>> {
        let mut pages: Vec<Vec<Op>> = Vec::new();
        for op in ops {
            if matches!(op, Op::ClearRect(0.0, 0.0, _, _)) {
                pages.push(Vec::new());
            } else if let Some(page) = pages.last_mut() {
                page.push(op.clone());
            }
        }
        pages
    }

    /// Text the page painted, paragraph marks dropped.
    fn text(page: &[Op]) -> String {
        page.iter()
            .filter_map(|op| match op {
                Op::FillText(text, ..) | Op::StrokeText(text, ..) if text != "\u{21b5}" => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect()
    }

    fn assert_blank(page: &[Op]) {
        assert!(
            !page
                .iter()
                .any(|op| matches!(op, Op::FillText(..) | Op::StrokeText(..))),
            "no text, header, footer or watermark: {page:?}"
        );
        assert!(
            !page.iter().any(|op| matches!(
                op,
                Op::SetStrokeStyle(color) if color.contains(RED) || color.contains(BLUE)
            )),
            "no page border: {page:?}"
        );
    }

    #[test]
    fn an_odd_page_break_inserts_a_blank_numbered_page() {
        let (renderer, _, pages) = render(&two_sections(paragraphs("a", 1), "oddPage"));
        assert_eq!(pages.len(), 3, "{pages:?}");
        let page_one = text(&pages[0]);
        assert!(
            page_one.contains("HA1") && page_one.contains("WM"),
            "{page_one}"
        );
        assert_blank(&pages[1]);
        assert_eq!(
            text(&pages[2]),
            "WMHB3P3FB3",
            "B's header, body PAGE field and footer on page 3"
        );
        assert!(
            pages[2]
                .iter()
                .any(|op| matches!(op, Op::SetStrokeStyle(color) if color.contains(BLUE))),
            "B's border on page 3"
        );
        assert_eq!(
            renderer
                .parity_filler_pages
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [2]
        );
    }

    #[test]
    fn an_even_page_break_inserts_a_blank_numbered_page() {
        let a_body = format!("{}{PAGE_BREAK}{}", paragraphs("a", 1), paragraphs("c", 1));
        let (_, _, pages) = render(&two_sections(a_body, "evenPage"));
        assert_eq!(pages.len(), 4, "{pages:?}");
        assert!(
            text(&pages[1]).contains("HA2"),
            "A's second page keeps its chrome"
        );
        assert_blank(&pages[2]);
        assert_eq!(text(&pages[3]), "WMHB4P4FB4");
    }

    #[test]
    fn a_break_already_on_the_right_parity_inserts_no_page() {
        let (renderer, _, pages) = render(&two_sections(paragraphs("a", 1), "evenPage"));
        assert_eq!(pages.len(), 2, "{pages:?}");
        assert_eq!(text(&pages[1]), "WMHB2P2FB2");
        assert!(renderer.parity_filler_pages.is_empty());
    }

    #[test]
    fn repainting_only_the_filler_page_keeps_it_blank() {
        let (mut renderer, document, _) = render(&two_sections(paragraphs("a", 1), "oddPage"));
        renderer.set_visible_page_range(Some(2..=2));
        renderer.prepare_rerender_for_page(1);
        renderer.host.painter().inner().clear();
        renderer
            .render(&document, &Options::default())
            .expect("rerender");
        let pages = split_pages(&renderer.host.painter().inner().ops());
        assert_eq!(pages.len(), 1, "only page 2 is repainted: {pages:?}");
        assert_blank(&pages[0]);
        assert_eq!(
            renderer
                .parity_filler_pages
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [2]
        );
    }

    #[test]
    fn a_relayout_that_no_longer_needs_the_filler_drops_it() {
        let (mut renderer, _, _) = render(&two_sections(paragraphs("a", 1), "oddPage"));
        assert!(renderer.parity_filler_pages.contains(&2));
        let word_doc = futures::executor::block_on(WordDocument::load(
            &two_sections(paragraphs("a", 1), "evenPage"),
            &Options::default(),
        ))
        .expect("load");
        renderer.sync_document_header_footer_stories(&word_doc);
        renderer.host.painter().inner().clear();
        renderer
            .render(
                word_doc.document.as_deref().expect("document"),
                &Options::default(),
            )
            .expect("render");
        assert!(renderer.parity_filler_pages.is_empty());
        let pages = split_pages(&renderer.host.painter().inner().ops());
        assert_eq!(text(&pages[1]), "HB2P2FB2");
    }
}
