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
    //! parity. Word prints it blank (no header, footer, page borders,
    //! watermark or manuscript grid) yet it takes a page number, as the Word
    //! for Mac capture of `section-break-types.docx` shows (an empty page 3
    //! between the evenPage break's source and target sections).

    use super::super::continuous_chrome_tests::{package, paragraphs, section, PAGE_FIELD};
    use super::super::recording_host::{recording_renderer, RecordingHost};
    use super::super::*;
    use crate::word_document::WordDocument;
    use jade_painter::recording::{Op, RecordingPainter};

    const RED: &str = "FF0000";
    const BLUE: &str = "0000FF";
    /// The manuscript grid's stroke colour.
    const GRID: &str = "00AA00";
    const PAGE_BREAK: &str = r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#;

    type Renderer = CanvasRenderer<RecordingPainter>;

    /// Section A's `a_body`, then section B starting with `break_type` and
    /// holding `b_body`. Both sections have headers and footers followed by
    /// a PAGE field, and page borders; `extra_a`/`extra_b` add section
    /// properties such as `w:pgNumType`.
    fn sections(
        a_body: String,
        extra_a: &str,
        break_type: &str,
        b_body: String,
        extra_b: &str,
    ) -> Vec<u8> {
        package(
            &[
                section("A", RED, "nextPage", a_body, extra_a),
                section("B", BLUE, break_type, b_body, extra_b),
            ],
            "",
        )
    }

    fn two_sections(a_body: String, break_type: &str) -> Vec<u8> {
        sections(a_body, "", break_type, PAGE_FIELD.to_string(), "")
    }

    /// Adds a text watermark and a manuscript grid to `document`.
    fn decorate(document: &mut Document) {
        document.watermark_text = Some("WM".into());
        document.manuscript_grid = Some(jade_docx_reader::document::ManuscriptGrid {
            style: jade_docx_reader::document::ManuscriptStyle::Lines,
            chars: 20,
            lines: 20,
            color: format!("#{GRID}"),
        });
    }

    /// Renders `data` decorated, returning the renderer, the rendered
    /// document and the ops of each page.
    fn render(data: &[u8]) -> (Renderer, Rc<Document>, Vec<Vec<Op>>) {
        let word_doc = futures::executor::block_on(WordDocument::load(data, &Options::default()))
            .expect("load");
        let mut document = Document::clone(word_doc.document.as_deref().expect("document"));
        decorate(&mut document);
        let document = Rc::new(document);
        let mut renderer = recording_renderer();
        renderer.sync_document_header_footer_stories(&word_doc);
        renderer
            .render(&document, &Options::default())
            .expect("render");
        let pages = split_pages(&renderer.host.painter().inner().ops());
        (renderer, document, pages)
    }

    /// The painted ops of each page, split at its full-page clear.
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

    fn strokes_with(page: &[Op], color: &str) -> bool {
        page.iter()
            .any(|op| matches!(op, Op::SetStrokeStyle(stroke) if stroke.contains(color)))
    }

    fn assert_blank(page: &[Op]) {
        assert!(
            !page
                .iter()
                .any(|op| matches!(op, Op::FillText(..) | Op::StrokeText(..))),
            "no text, header, footer or watermark: {page:?}"
        );
        assert!(
            !strokes_with(page, RED) && !strokes_with(page, BLUE),
            "no page border: {page:?}"
        );
        assert!(!strokes_with(page, GRID), "no manuscript grid: {page:?}");
    }

    fn fillers(renderer: &Renderer) -> Vec<usize> {
        renderer.parity_filler_pages.iter().copied().collect()
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
        assert!(strokes_with(&pages[2], BLUE), "B's border on page 3");
        assert!(strokes_with(&pages[2], GRID), "the grid on page 3");
        assert_eq!(fillers(&renderer), [2]);
        assert!(renderer.is_parity_filler_page_index(1));
        assert!(!renderer.is_parity_filler_page_index(0));
        assert!(!renderer.is_parity_filler_page_index(2));
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
        assert_eq!(fillers(&renderer), [2]);
    }

    #[test]
    fn a_relayout_that_no_longer_needs_the_filler_drops_it() {
        let (mut renderer, _, _) = render(&two_sections(paragraphs("a", 1), "oddPage"));
        assert_eq!(fillers(&renderer), [2]);
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

    /// A seek keeps the entries of the pages it skips and settles those it
    /// walks afresh.
    #[test]
    fn a_seek_keeps_the_skipped_pages_entries_and_settles_the_walked_ones() {
        // B's pages 3 to 7 each start with a paragraph at the page top.
        let b_body: String = (0..5)
            .map(|index| {
                format!(r#"<w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>b{index}</w:t></w:r></w:p>"#)
            })
            .collect();
        let data = sections(paragraphs("a", 1), "", "oddPage", b_body, "");
        let (mut renderer, document, _) = render(&data);
        let page_count = renderer.page_section_indices.len();
        assert_eq!(page_count, 7);
        assert_eq!(fillers(&renderer), [2]);
        let snapshot = renderer
            .page_start_seek_cache_by_page
            .borrow()
            .iter()
            .filter(|(&page, snapshot)| page >= 4 && page < page_count && !snapshot.spanning)
            .min_by_key(|(&page, _)| page)
            .map(|(_, snapshot)| snapshot.clone())
            .unwrap_or_else(|| {
                panic!(
                    "no page-top snapshot on page 4..{page_count}: {:?} {:?}",
                    renderer
                        .page_start_seek_cache_by_page
                        .borrow()
                        .iter()
                        .map(|(page, snapshot)| (*page, snapshot.page_number, snapshot.spanning))
                        .collect::<Vec<_>>(),
                    renderer.paragraph_seek_cache.borrow().len()
                )
            });
        let resume = snapshot.page_number;
        // Page 1 is not a filler: only a walk that skips it keeps the entry.
        // The page after the resume page is walked, so it is settled anew.
        renderer.parity_filler_pages.insert(1);
        renderer.parity_filler_pages.insert(resume + 1);
        renderer.set_visible_page_range(Some(resume..=page_count));
        renderer.forced_seek_snapshot = Some(snapshot);
        renderer
            .render(&document, &Options::default())
            .expect("seek render");
        assert_eq!(fillers(&renderer), [1, 2], "resumed at page {resume}");
    }

    /// The outgoing section's footnote continues onto the page an oddPage
    /// break would otherwise leave blank. On Mac the parity page carries
    /// it, so it is no filler and keeps its chrome; Windows places the
    /// continuation first and then settles parity from there.
    #[test]
    fn a_page_carrying_notes_is_no_filler() {
        use jade_docx_layout::line_metrics::{
            set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
        };
        struct Restore(WordLayoutPlatform);
        impl Drop for Restore {
            fn drop(&mut self) {
                set_word_layout_platform(self.0);
            }
        }
        let _restore = Restore(word_layout_platform());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures/footnote-long-continuation.docx");
        let bytes = std::fs::read(path).expect("read fixture");
        let model = futures::executor::block_on(WordDocument::load(&bytes, &Options::default()))
            .expect("load fixture");
        let mut document = Document::clone(model.document.as_deref().expect("document"));
        decorate(&mut document);
        // Split after "Reference to a long note" (element 30): the note's
        // continuation is still pending when the oddPage break is reached.
        let whole = document.sections.first().cloned().expect("one section");
        let mut incoming = whole.properties.clone().unwrap_or_default();
        incoming.section_type = Some("oddPage".into());
        document.sections = vec![
            SectionRange {
                start: whole.start,
                end: 31,
                properties: whole.properties.clone(),
            },
            SectionRange {
                start: 31,
                end: whole.end,
                properties: Some(incoming),
            },
        ];
        for platform in [WordLayoutPlatform::Mac, WordLayoutPlatform::Windows] {
            set_word_layout_platform(platform);
            let painter = RecordingPainter::new();
            let mut renderer = CanvasRenderer::with_host(
                Box::new(RecordingHost {
                    painter: painter.clone(),
                }),
                false,
            )
            .expect("paged recording renderer");
            renderer
                .render(&document, &Options::default())
                .expect("render");
            let pages = split_pages(&painter.ops());
            assert!(pages.len() >= 3, "{platform:?}: {} pages", pages.len());
            assert!(
                renderer.parity_filler_pages.is_empty(),
                "{platform:?}: {:?}",
                renderer.parity_filler_pages
            );
            let page_two = text(&pages[1]);
            assert!(
                page_two.contains("WM") && page_two.len() > 2,
                "{platform:?}: page 2 keeps its watermark and notes: {page_two}"
            );
            assert!(
                strokes_with(&pages[1], GRID),
                "{platform:?}: grid on page 2"
            );
        }
    }
}

mod parity_filler_numbering {
    //! Word tests an oddPage/evenPage break against the number the page
    //! displays (`w:pgNumType`), not its physical position.

    use super::super::continuous_chrome_tests::{package, paragraphs, section, PAGE_FIELD};
    use super::super::recording_host::recording_renderer;
    use super::super::*;
    use crate::word_document::WordDocument;
    use jade_painter::recording::RecordingPainter;

    fn render(extra_a: &str, break_type: &str, extra_b: &str) -> CanvasRenderer<RecordingPainter> {
        let data = package(
            &[
                section("A", "FF0000", "nextPage", paragraphs("a", 1), extra_a),
                section("B", "0000FF", break_type, PAGE_FIELD.to_string(), extra_b),
            ],
            "",
        );
        let word_doc = futures::executor::block_on(WordDocument::load(&data, &Options::default()))
            .expect("load");
        let mut renderer = recording_renderer();
        renderer.sync_document_header_footer_stories(&word_doc);
        renderer
            .render(
                word_doc.document.as_deref().expect("document"),
                &Options::default(),
            )
            .expect("render");
        renderer
    }

    /// Physical pages, filler pages, and the number B's first (last) page
    /// shows.
    fn layout(renderer: &CanvasRenderer<RecordingPainter>) -> (usize, Vec<usize>, usize) {
        let pages = renderer.page_section_indices.len();
        (
            pages,
            renderer.parity_filler_pages.iter().copied().collect(),
            renderer.page_number_value(pages),
        )
    }

    #[test]
    fn a_first_section_numbered_from_two_settles_parity_by_its_numbers() {
        let start_at_two = r#"<w:pgNumType w:start="2"/>"#;
        // Page 1 shows 2, so B's first page shows 3: already odd.
        assert_eq!(layout(&render(start_at_two, "oddPage", "")), (2, vec![], 3));
        assert_eq!(
            layout(&render(start_at_two, "evenPage", "")),
            (3, vec![2], 4)
        );
    }

    #[test]
    fn a_section_restarting_its_numbers_settles_parity_by_its_start() {
        // An odd-numbered page is a right-hand page: a restarting section
        // needs a blank first when its start shares the parity of the page
        // before the break, whichever parity the break names.
        let start = |n: u32| format!(r#"<w:pgNumType w:start="{n}"/>"#);
        // Front matter on page 1, then chapter 1 restarting at 1: 1, blank, 1.
        assert_eq!(layout(&render("", "oddPage", &start(1))), (3, vec![2], 1));
        assert_eq!(layout(&render("", "oddPage", &start(2))), (2, vec![], 2));
        assert_eq!(layout(&render("", "evenPage", &start(1))), (3, vec![2], 1));
        assert_eq!(layout(&render("", "evenPage", &start(2))), (2, vec![], 2));
        // Page 1 shows 2, so a section restarting at 2 needs the blank.
        assert_eq!(
            layout(&render(&start(2), "evenPage", &start(2))),
            (3, vec![2], 2)
        );
    }
}
