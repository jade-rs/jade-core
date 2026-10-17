//! End-to-end: a real DOCX becomes pixels with no browser in the process.
//!
//! Everything between the file and the PNG is `jade-docx-editor`'s own renderer —
//! the same code the browser runs. Only the surface and the text measurement
//! come from this crate, which is the whole claim being tested.

use futures::executor::block_on;
use jade_docx_editor::canvas_renderer::CanvasRenderer;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_native_render::SkiaRenderHost;
use jade_skia_painter::SkiaPainter;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Renders `fixture` and hands back every page's surface.
fn render(fixture: &str) -> Rc<RefCell<Vec<SkiaPainter>>> {
    let bytes = std::fs::read(repo_path(fixture)).expect("read fixture");
    let mut options = Options::default();
    options.show_formatting_marks = false;

    let word_document = block_on(WordDocument::load(&bytes, &options)).expect("parse docx");
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
    let mut renderer =
        CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false).expect("build renderer");
    renderer.render(document, &options).expect("render");
    pages
}

/// Counts pixels that are not the white the surface starts as.
fn inked_pixels(page: &SkiaPainter, step: i32) -> usize {
    let (_, width, height) = page.read_bgra_premultiplied().expect("page pixels");
    let mut inked = 0;
    let mut y = 0;
    while y < height {
        let mut x = 0;
        while x < width {
            if let Some((r, g, b, _)) = page.pixel(x, y) {
                if r < 200 || g < 200 || b < 200 {
                    inked += 1;
                }
            }
            x += step;
        }
        y += step;
    }
    inked
}

#[test]
fn a_docx_renders_to_pixels_without_a_browser() {
    let pages = render("public/fixtures/contract.docx");
    let pages = pages.borrow();

    // The browser renderer paginates this fixture to three pages; the native
    // one uses the same layout code, so it has to agree. A mismatch would mean
    // measurement, not drawing, had drifted.
    assert_eq!(pages.len(), 3, "expected three pages, got {}", pages.len());

    // Enough ink to be text rather than a stray rule or an empty page.
    let inked = inked_pixels(&pages[0], 4);
    assert!(inked > 500, "page 1 looks blank: {inked} inked samples");
}

/// The failure this guards against is specific: a host whose pages shared one
/// surface would paint page 2 on top of page 1 and still produce plausible
/// PNGs. Each page has to be its own surface.
#[test]
fn each_page_gets_its_own_surface() {
    let pages = render("public/fixtures/contract.docx");
    let pages = pages.borrow();

    // Sampled across the whole page rather than one row: most rows are white
    // on both pages, so a single row proves nothing either way.
    let signature = |page: &SkiaPainter| -> Vec<(u8, u8, u8, u8)> {
        let (_, width, height) = page.read_bgra_premultiplied().expect("page pixels");
        let mut out = Vec::new();
        let mut y = 0;
        while y < height {
            let mut x = 0;
            while x < width {
                out.push(page.pixel(x, y).expect("read"));
                x += 8;
            }
            y += 8;
        }
        out
    };
    assert_ne!(
        signature(&pages[0]),
        signature(&pages[1]),
        "pages 1 and 2 are pixel-identical"
    );

    // And a later page must have been given the fonts too — a page created
    // mid-render starts on a fresh Skia surface with none registered, which
    // would lay out correctly and then draw nothing.
    let inked = inked_pixels(&pages[1], 4);
    assert!(inked > 500, "page 2 looks blank: {inked} inked samples");
}
