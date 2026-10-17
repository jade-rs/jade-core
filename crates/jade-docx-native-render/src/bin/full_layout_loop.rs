//! Loads a DOCX and lays out the whole document from cold caches, repeatedly,
//! so a sampling profiler has the browser's "complete lazy layout" path to
//! attach to.
//!
//! Usage: `full_layout_loop <input.docx> [iterations]`
//!
//! Only page 1 is drawn, as in the browser, where offscreen pages are laid out
//! but not painted. Each iteration builds a fresh renderer so the paragraph
//! wrap and text-measure caches start empty; parsing is timed separately.

use futures::executor::block_on;
use jade_docx_editor::canvas_renderer::CanvasRenderer;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_native_render::SkiaRenderHost;
use jade_skia_painter::SkiaPainter;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;

fn bundled_font(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fonts/Noto_Sans_JP/static")
        .join(name)
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .ok_or_else(|| "usage: full_layout_loop <input.docx> [iterations]".to_string())?;
    let iterations: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(20);
    let bytes = std::fs::read(&input).map_err(|err| format!("{input}: {err}"))?;
    let mut options = Options::default();
    options.show_formatting_marks = false;

    let mut parse_total = Duration::ZERO;
    let mut layout_total = Duration::ZERO;
    let mut pages = 0;
    for _ in 0..iterations {
        let started = Instant::now();
        let word_document = block_on(WordDocument::load(&bytes, &options))
            .map_err(|err| format!("{input}: parse failed: {err}"))?;
        parse_total += started.elapsed();
        let document = word_document
            .document
            .as_ref()
            .ok_or_else(|| format!("{input}: no main document part"))?;

        let mut host = SkiaRenderHost::new(PAGE_WIDTH_PX, PAGE_HEIGHT_PX)
            .ok_or_else(|| "could not allocate the raster surface".to_string())?;
        for (weight, file) in [
            (400, "NotoSansJP-Regular.ttf"),
            (700, "NotoSansJP-Bold.ttf"),
        ] {
            for family in [
                "Noto Sans JP",
                "Times New Roman",
                "Yu Mincho",
                "MS Mincho",
                "MS Gothic",
                "serif",
            ] {
                host.register_font(family, weight, bundled_font(file))?;
            }
        }
        let mut renderer = CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false)
            .map_err(|_| "renderer setup failed".to_string())?;
        renderer.set_visible_page_range(Some(1..=1));

        let started = Instant::now();
        renderer
            .render(document, &options)
            .map_err(|_| "render failed".to_string())?;
        layout_total += started.elapsed();
        pages = renderer.page_count();
    }
    let per = |total: Duration| total.as_secs_f64() * 1000.0 / iterations as f64;
    println!(
        "{input}: {pages} pages, parse {:.1} ms, layout {:.1} ms (mean of {iterations})",
        per(parse_total),
        per(layout_total)
    );
    Ok(())
}
