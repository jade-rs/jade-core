//! Typing in a numbered paragraph repaints only that paragraph's band.
//!
//! Contracts are mostly numbered paragraphs. The band path repaints the edited
//! paragraph's slice of its page and keeps every other pixel, so the page has
//! to come out exactly as a full repaint of the edited document draws it —
//! the numbering marker included, since the band clears it with the paragraph.

use futures::executor::block_on;
use jade_docx_editor::canvas_renderer::CanvasRenderer;
use jade_docx_editor::headless_api::{
    apply_headless_commands_to_document, execute_headless_queries_on_document,
};
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_headless_api::{
    HeadlessCommand, HeadlessDeleteDirection, HeadlessEditRequest, HeadlessQuery,
    HeadlessQueryRequest, HeadlessQueryResult,
};
use jade_docx_native_render::SkiaRenderHost;
use jade_skia_painter::SkiaPainter;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;

type Pages = Rc<RefCell<Vec<SkiaPainter>>>;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn options() -> Options {
    let mut options = Options::default();
    options.show_formatting_marks = false;
    options
}

fn renderer() -> (CanvasRenderer<SkiaPainter>, Pages) {
    let mut host = SkiaRenderHost::new(PAGE_WIDTH_PX, PAGE_HEIGHT_PX).expect("raster surface");
    for (weight, file) in [
        (400, "NotoSansJP-Regular.ttf"),
        (700, "NotoSansJP-Bold.ttf"),
    ] {
        let path = repo_path(&format!("fonts/Noto_Sans_JP/static/{file}"));
        for family in [
            "Noto Sans JP",
            "Times New Roman",
            "Yu Mincho",
            "MS Mincho",
            "serif",
        ] {
            host.register_font(family, weight, &path)
                .expect("register face");
        }
    }
    let pages = host.pages();
    let renderer =
        CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false).expect("build renderer");
    (renderer, pages)
}

fn render_everything(renderer: &mut CanvasRenderer<SkiaPainter>, word: &WordDocument) {
    renderer
        .render_with_numbering_rc(
            word.document.clone().expect("main document"),
            &options(),
            word.numbering_part.as_ref(),
            word.header_lines.as_deref(),
            word.footer_lines.as_deref(),
            None,
        )
        .expect("render");
}

fn page_pixels(pages: &Pages, index: usize) -> Vec<u8> {
    pages.borrow()[index]
        .read_bgra_premultiplied()
        .expect("read pixels")
        .0
}

fn paragraph_text(word: &WordDocument, paragraph: usize) -> Option<String> {
    let response = execute_headless_queries_on_document(
        word,
        &HeadlessQueryRequest {
            queries: vec![HeadlessQuery::ParagraphSlice {
                start_paragraph: paragraph,
                max_paragraphs: 1,
            }],
        },
    )
    .ok()?;
    response
        .results
        .into_iter()
        .find_map(|result| match result {
            HeadlessQueryResult::ParagraphSlice { paragraphs, .. } => paragraphs
                .into_iter()
                .find(|entry| entry.paragraph == paragraph)
                .map(|entry| entry.text),
            _ => None,
        })
}

fn taken(renderer: &CanvasRenderer<SkiaPainter>) -> u64 {
    renderer.band_redraw_stats().1
}

#[test]
fn typing_in_a_numbered_paragraph_repaints_its_band_and_matches_a_full_repaint() {
    let bytes = std::fs::read(repo_path("public/fixtures/contract.docx")).expect("read fixture");
    let mut word = block_on(WordDocument::load(&bytes, &options())).expect("parse docx");
    let (mut renderer, pages) = renderer();
    render_everything(&mut renderer, &word);

    // A short numbered paragraph on page 1: a few characters more neither
    // wrap it nor move anything below it, so every keystroke can take the band.
    let paragraph = renderer
        .numbering_debug_regions()
        .iter()
        .filter(|marker| marker.page == 1)
        .map(|marker| marker.paragraph)
        .find(|&paragraph| {
            paragraph_text(&word, paragraph)
                .is_some_and(|text| (4..=12).contains(&text.chars().count()))
        })
        .expect("a short numbered paragraph on page 1");
    let page = 1;

    // Typing at the end, typing right after the marker (the character the
    // marker is laid out against changes), and a Backspace.
    let keystrokes = [
        |text: &str, paragraph| HeadlessCommand::InsertText {
            paragraph,
            offset: text.len(),
            text: "あ".to_string(),
        },
        |_: &str, paragraph| HeadlessCommand::InsertText {
            paragraph,
            offset: 0,
            text: "い".to_string(),
        },
        |text: &str, paragraph| HeadlessCommand::DeleteText {
            paragraph,
            offset: text.len(),
            direction: HeadlessDeleteDirection::Backward,
            count: 1,
        },
    ];
    let before = taken(&renderer);
    for keystroke in keystrokes {
        let text = paragraph_text(&word, paragraph).expect("paragraph text");
        apply_headless_commands_to_document(
            &mut word,
            &HeadlessEditRequest {
                commands: vec![keystroke(&text, paragraph)],
            },
        )
        .expect("apply the edit");
        // The browser shell's sequence for an edit: name the paragraph, then
        // repaint through its page with band redraw on.
        renderer.prepare_incremental_repaint(paragraph, page);
        renderer.set_band_redraw_enabled(true);
        renderer
            .render_bounded_to_page_with_parts(
                word.document.clone().expect("main document"),
                &options(),
                word.numbering_part.as_ref(),
                word.header_lines.as_deref(),
                word.footer_lines.as_deref(),
                page,
            )
            .expect("repaint");
        renderer.set_band_redraw_enabled(false);
    }
    let (_, _, fallbacks) = renderer.band_redraw_stats();
    assert_eq!(
        taken(&renderer) - before,
        keystrokes.len() as u64,
        "every keystroke in numbered paragraph {paragraph} should take the band; \
         fallbacks: {fallbacks:?}"
    );

    let (_fresh, fresh_pages) = renderer_and_render(&word);
    let typed = page_pixels(&pages, page - 1);
    let expected = page_pixels(&fresh_pages, page - 1);
    assert_eq!(typed.len(), expected.len());
    let differing = typed
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differing, 0,
        "page {page} after typing differs from a full repaint in {differing} pixels"
    );
}

fn renderer_and_render(word: &WordDocument) -> (CanvasRenderer<SkiaPainter>, Pages) {
    let (mut renderer, pages) = renderer();
    render_everything(&mut renderer, word);
    (renderer, pages)
}
