//! What does one keystroke cost, natively?
//!
//! `CanvasRenderer::render` is the "open the document" path: it repaints
//! everything, and re-rendering an unchanged document through it costs the same
//! as a cold render. The browser is usable for typing because `canvas_view`
//! runs a different sequence after an edit — it tells the renderer *which
//! paragraph* changed before asking it to draw.
//!
//! This reproduces that sequence natively, so the number below is a keystroke
//! rather than a document open.

use futures::executor::block_on;
use jade_docx_editor::canvas_renderer::CanvasRenderer;
use jade_docx_editor::headless_api::apply_headless_commands_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_docx_native_render::SkiaRenderHost;
use jade_skia_painter::SkiaPainter;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;
/// A paragraph on the page this repaint is bounded to.
///
/// It has to be. Editing a paragraph on page 2 while bounding layout to page 1
/// gives a repaint that redraws page 1 in full, at a plausible speed, showing
/// the same pixels forever — the earlier version of this benchmark used
/// paragraph 30 and timed exactly that.
const EDIT_PARAGRAPH: usize = 6;
/// The last page the window is showing, 1-based. Typing should only cost the
/// pages a reader can actually see.
const VISIBLE_END_PAGE_1IDX: usize = 1;

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

fn host_with_fonts() -> SkiaRenderHost {
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
    host
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort();
    values[values.len() / 2]
}

#[test]
fn one_keystroke_costs_far_less_than_opening_the_document() {
    let bytes = std::fs::read(repo_path("public/fixtures/contract.docx")).expect("read fixture");
    let mut word_document = block_on(WordDocument::load(&bytes, &options())).expect("parse docx");

    let host = host_with_fonts();
    let pages = host.pages();
    let measured = host.measurement_handle();
    let mut renderer =
        CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false).expect("build renderer");
    // `set_band_redraw_enabled` is deliberately not called here. It is off by
    // default, and it looks like the optimisation that should make typing
    // cheap: only the band around the edited paragraph is repainted, and the
    // rest of the page keeps the pixels already on the surface — which this
    // host preserves. Measured alternating on and off, three runs each, it made
    // no difference this setup could resolve: on gave 133/160/157ms, off gave
    // 226/151/149ms. Either it is not engaging without state this host does not
    // provide, or the cost is not where it saves. Left out rather than shipped
    // as an unverified win.

    // The document open: everything laid out and painted from nothing.
    let document = word_document
        .document
        .as_ref()
        .expect("main document")
        .clone();
    let open = Instant::now();
    renderer
        .render(&document, &options())
        .expect("first render");
    let open = open.elapsed();
    drop(document);

    // A bounded repaint is only worth timing if it actually draws the edit.
    // Limiting layout to the visible page while editing a paragraph that sits
    // on a later one would produce a fast number for doing nothing.
    let page_pixels = |index: usize| -> Vec<u8> {
        pages.borrow()[index]
            .read_bgra_premultiplied()
            .expect("read pixels")
            .0
    };
    let before_typing = page_pixels(VISIBLE_END_PAGE_1IDX - 1);
    let ink = |pixels: &[u8]| pixels.chunks_exact(4).filter(|px| px[0] < 200).count();
    let ink_before = ink(&before_typing);

    // Then twenty keystrokes, each one edit plus the repaint sequence
    // `canvas_view` runs after an edit.
    let mut keystrokes = Vec::new();
    // How much text each repaint has to measure. A keystroke that measures as
    // much as a document open is laying out the whole document, whatever the
    // page limit says.
    let mut per_keystroke_measures: Vec<u64> = Vec::new();
    for _ in 0..20 {
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

        // The height the paragraph had before the edit, which is how the
        // renderer decides whether the content below it moved.
        let before_calls = measured().0;
        let started = Instant::now();
        // The sequence, in `canvas_view`'s order.
        //
        // `set_incremental_dirty_from` names the changed paragraph and turns
        // off hash-based change detection, which lets the renderer skip the
        // paragraphs it already has. The *bounded* dirty marking and the layout
        // page limit are what stop the work running to the end of the document:
        // typing on page 1 should not re-flow page 3 before the frame is shown.
        // `set_visible_page_range` belongs here in the browser's sequence and is
        // deliberately absent: with it, this host's repaint draws *nothing* —
        // the page comes back pixel-identical and the assertion below catches
        // it. The browser pairs a narrowed visible range with its page-bitmap
        // cache, which this host stubs out, so the renderer skips the draw and
        // there is no bitmap to composite in its place. Closing the gap to the
        // browser's keystroke cost means giving the host that cache, not adding
        // this call.
        renderer.prefer_visible_window_seek_once();
        renderer.set_incremental_dirty_from(EDIT_PARAGRAPH);
        renderer.invalidate_paragraph_wrap_cache_from(EDIT_PARAGRAPH);
        renderer.mark_dirty_from_paragraph_to_end_bounded(EDIT_PARAGRAPH, VISIBLE_END_PAGE_1IDX);
        let previous_limit = renderer.layout_page_limit();
        renderer.set_layout_page_limit(Some(VISIBLE_END_PAGE_1IDX));
        renderer
            .render_with_numbering_rc(edited, &options(), None, None, None, None)
            .expect("repaint");
        renderer.set_layout_page_limit(previous_limit);
        keystrokes.push(started.elapsed());
        per_keystroke_measures.push(measured().0 - before_calls);
    }

    // Ink, not "some pixel differs": a repaint can redraw a band identically
    // and still differ, which is how a benchmark that timed nothing passed.
    let ink_after = ink(&page_pixels(VISIBLE_END_PAGE_1IDX - 1));
    assert!(
        ink_after > ink_before,
        "20 keystrokes added no ink ({ink_before} -> {ink_after}) — the repaint \
         is not drawing the edit, so its timing means nothing"
    );

    // The interesting result is that these are zero: a keystroke re-measures no
    // text at all, so the renderer's measurement caches are working. Combined
    // with text drawing being ~1.3% of a repaint, a ~113ms keystroke is neither
    // measuring nor drawing — it is bookkeeping. That is a smell in how this
    // test drives the renderer, not a property of rendering natively.
    let open_measures = measured().0 - per_keystroke_measures.iter().sum::<u64>();
    println!(
        "measurement calls: document open={open_measures}, per keystroke={:?}",
        &per_keystroke_measures[..5.min(per_keystroke_measures.len())]
    );

    let keystroke = median(keystrokes.clone());
    println!("document open: {open:?}");
    println!(
        "keystroke repaint: median={keystroke:?} min={:?} max={:?} over {} edits",
        keystrokes.iter().min().unwrap(),
        keystrokes.iter().max().unwrap(),
        keystrokes.len()
    );

    // The claim under test is only that naming the changed paragraph buys
    // something. How much is reported above rather than asserted, since it is
    // a machine-dependent number.
    assert!(
        keystroke < open,
        "a keystroke ({keystroke:?}) should cost less than opening the document ({open:?})"
    );
}
