//! Repeats the keystroke repaint forever so a sampling profiler has something
//! to attach to.
//!
//! Not a benchmark — `tests/keystroke_cost.rs` is. This exists because the
//! timings there say a repaint neither measures text nor spends its time
//! drawing, which leaves ~98% unaccounted for. A call tree is the way to find
//! it; guessing at renderer flags was not.

use futures::executor::block_on;
use jade_docx_editor::canvas_renderer::CanvasRenderer;
use jade_docx_editor::headless_api::apply_headless_commands_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_docx_native_render::SkiaRenderHost;
use jade_skia_painter::SkiaPainter;
use std::path::PathBuf;

const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;
const EDIT_PARAGRAPH: usize = 30;
const VISIBLE_END_PAGE_1IDX: usize = 1;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn main() {
    let mut options = Options::default();
    options.show_formatting_marks = false;

    let bytes = std::fs::read(repo_path("public/fixtures/contract.docx")).expect("read fixture");
    let mut word_document = block_on(WordDocument::load(&bytes, &options)).expect("parse docx");

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
    let mut renderer =
        CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false).expect("build renderer");

    let document = word_document
        .document
        .as_ref()
        .expect("main document")
        .clone();
    renderer.render(&document, &options).expect("first render");
    drop(document);

    eprintln!("ready, pid {}", std::process::id());
    loop {
        apply_headless_commands_to_document(
            &mut word_document,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertText {
                    paragraph: EDIT_PARAGRAPH,
                    offset: 0,
                    text: "あ".to_string(),
                }],
            },
        )
        .expect("apply the edit");
        let edited = word_document
            .document
            .as_ref()
            .expect("main document")
            .clone();

        renderer.prefer_visible_window_seek_once();
        renderer.set_incremental_dirty_from(EDIT_PARAGRAPH);
        renderer.invalidate_paragraph_wrap_cache_from(EDIT_PARAGRAPH);
        renderer.mark_dirty_from_paragraph_to_end_bounded(EDIT_PARAGRAPH, VISIBLE_END_PAGE_1IDX);
        let previous_limit = renderer.layout_page_limit();
        renderer.set_layout_page_limit(Some(VISIBLE_END_PAGE_1IDX));
        renderer
            .render_with_numbering_rc(edited, &options, None, None, None, None)
            .expect("repaint");
        renderer.set_layout_page_limit(previous_limit);
    }
}
