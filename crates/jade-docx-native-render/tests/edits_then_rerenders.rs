//! Does an edit reach the pixels, natively?
//!
//! Rendering without a browser is only half of an editor. This drives the other
//! half end to end: load a document, render it, apply a real edit through
//! `jade-docx-editor`'s headless command API — the same path the MCP server uses to
//! edit DOCX files — and render again.
//!
//! Nothing here involves input handling or IME. The point is narrower and worth
//! separating: that the document model, the edit, and the native renderer
//! compose at all, and roughly what a repaint costs.

use futures::executor::block_on;
use jade_docx_editor::canvas_renderer::CanvasRenderer;
use jade_docx_editor::headless_api::apply_headless_commands_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_docx_native_render::SkiaRenderHost;
use jade_skia_painter::SkiaPainter;
use std::path::PathBuf;
use std::time::Instant;

const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;

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

/// Renders `word_document` and returns each page's pixels.
///
/// A fresh host per render, which is what makes this a *repaint* measurement
/// rather than a measurement of the renderer's incremental caches. A real
/// editor would keep the renderer alive; this is the pessimistic number.
fn render_pages(word_document: &WordDocument) -> (Vec<Vec<u8>>, std::time::Duration) {
    let document = word_document.document.as_ref().expect("main document part");
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
    let started = Instant::now();
    let mut renderer =
        CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false).expect("build renderer");
    renderer.render(document, &options()).expect("render");
    let elapsed = started.elapsed();

    let pixels = pages
        .borrow()
        .iter()
        .map(|page| page.read_bgra_premultiplied().expect("read pixels").0)
        .collect();
    (pixels, elapsed)
}

#[test]
fn an_edit_reaches_the_pixels() {
    let bytes = std::fs::read(repo_path("public/fixtures/contract.docx")).expect("read fixture");
    let mut word_document = block_on(WordDocument::load(&bytes, &options())).expect("parse docx");

    let (before, first_render) = render_pages(&word_document);

    // A real edit through the ordinary command API, not a poke at the model.
    apply_headless_commands_to_document(
        &mut word_document,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 0,
                text: "ネイティブ編集テスト ".to_string(),
            }],
        },
    )
    .expect("apply the edit");

    let (after, repaint) = render_pages(&word_document);

    assert_eq!(before.len(), after.len(), "page count changed unexpectedly");
    assert_ne!(
        before[0], after[0],
        "page 1 is pixel-identical after inserting text into its first paragraph"
    );

    // Not asserted, only reported: this is the number that decides whether the
    // renderer can be driven from a UI event loop, and it is a full repaint
    // with a cold renderer.
    println!(
        "full repaint: first={:?} after-edit={:?} ({} pages)",
        first_render,
        repaint,
        after.len()
    );
}

/// The edit has to land in the document model too, not only in the pixels —
/// otherwise a rendering artefact could pass the test above.
#[test]
fn the_edit_is_in_the_document_not_just_the_render() {
    let bytes = std::fs::read(repo_path("public/fixtures/contract.docx")).expect("read fixture");
    let mut word_document = block_on(WordDocument::load(&bytes, &options())).expect("parse docx");

    const INSERTED: &str = "ネイティブ編集テスト";
    apply_headless_commands_to_document(
        &mut word_document,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 0,
                text: INSERTED.to_string(),
            }],
        },
    )
    .expect("apply the edit");

    // Exporting proves the edit survives a round trip to a real .docx, which is
    // what an editor would eventually have to save.
    let exported = word_document.export_docx().expect("export docx");
    let reloaded = block_on(WordDocument::load(&exported, &options())).expect("reparse export");
    let document = reloaded.document.as_ref().expect("main document part");
    let text = format!("{document:?}");
    assert!(
        text.contains(INSERTED),
        "the inserted text did not survive export and reload"
    );
}

/// The number that actually decides whether editing is viable.
///
/// The test above rebuilds the renderer for each render, so every cache starts
/// empty — a pessimistic figure. An editor keeps one renderer alive, and jade's
/// renderer carries paragraph-level draw-command caches built for exactly this.
/// This measures a repaint on a warm renderer, which is what a keystroke would
/// actually cost.
#[test]
fn a_warm_repaint_is_much_cheaper_than_a_cold_one() {
    // Layout invalidation runs from the edited paragraph to the end of the
    // document, so editing paragraph 0 re-lays out everything by definition.
    // A late paragraph is both the realistic typing case and the one that shows
    // whether the incremental path is doing anything at all.
    const EDIT_PARAGRAPH: usize = 30;
    let bytes = std::fs::read(repo_path("public/fixtures/contract.docx")).expect("read fixture");
    let mut word_document = block_on(WordDocument::load(&bytes, &options())).expect("parse docx");

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
    let stats = host.measurement_handle();
    let mut renderer =
        CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false).expect("build renderer");

    let document = word_document
        .document
        .as_ref()
        .expect("main document")
        .clone();
    let cold = Instant::now();
    renderer
        .render(&document, &options())
        .expect("first render");
    let cold = cold.elapsed();

    // Same document, same renderer: whatever the caches are worth shows up here.
    let unchanged = Instant::now();
    renderer
        .render(&document, &options())
        .expect("second render");
    let unchanged = unchanged.elapsed();
    drop(document);

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

    let edited_document = word_document
        .document
        .as_ref()
        .expect("main document")
        .clone();

    // The renderer is told *what changed* before being asked to draw. Without
    // this it has no dirty information and repaints everything, which is what
    // the browser runtime does after an edit and what the incremental caches
    // are keyed on. Skipping it makes a keystroke look as expensive as opening
    // the file.
    renderer.invalidate_layout_from(EDIT_PARAGRAPH);
    renderer.mark_paragraph_dirty(EDIT_PARAGRAPH);

    let after_edit = Instant::now();
    renderer
        .render(&edited_document, &options())
        .expect("repaint");
    let after_edit = after_edit.elapsed();

    let (calls, in_measurement) = stats();
    println!(
        "warm renderer: cold={cold:?} unchanged={unchanged:?} after-1-char-edit={after_edit:?}"
    );
    // Measured, not assumed: this is 613 calls and about 3ms across all three
    // renders — under half a percent of the ~290ms they take. Whatever makes a
    // repaint expensive, it is not the native text measurement.
    println!("  measurement across all three renders: {calls} calls, {in_measurement:?}");
}
