//! Renders a DOCX to one PNG per page, with no browser involved.
//!
//! Usage: `docx_to_png [--real-fonts] <input.docx> <output.png>` — later
//! pages are written beside the first as `<output>-2.png`, `<output>-3.png`,
//! and so on. With `--real-fonts` the document's own fonts are used where
//! they are installed (Office's own first), as the native app draws it;
//! without, every face is the bundled Noto Sans JP.
//!
//! This is the end-to-end check that the renderer is no longer tied to a
//! canvas. Everything it does — parsing, layout, wrapping, pagination, drawing
//! — is `jade-docx-editor`'s own code, unchanged; only the surface and the text
//! measurement are supplied by this crate.

use futures::executor::block_on;
use jade_docx_editor::canvas_renderer::CanvasRenderer;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_native_render::SkiaRenderHost;
use jade_skia_painter::SkiaPainter;
use std::path::{Path, PathBuf};

/// US Letter at 96 dpi, which is what the browser renderer uses as its
/// starting page box before the document's own section properties apply.
const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;

fn bundled_font(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fonts/Noto_Sans_JP/static")
        .join(name)
}

fn main() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let real_fonts = args.iter().any(|arg| arg == "--real-fonts");
    args.retain(|arg| arg != "--real-fonts");
    let mut args = args.into_iter();
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        return Err("usage: docx_to_png [--real-fonts] <input.docx> <output.png>".to_string());
    };

    let bytes = std::fs::read(&input).map_err(|err| format!("{input}: {err}"))?;
    let mut options = Options::default();
    // Formatting marks are a view setting, not document content, and the
    // bundled face has no pilcrow — leaving them on fills the page with tofu.
    options.show_formatting_marks = false;

    // `OpenXmlPackage::load` is `async` but never awaits anything outside the
    // process, so blocking on it is sound off the browser.
    let word_document = block_on(WordDocument::load(&bytes, &options))
        .map_err(|err| format!("{input}: parse failed: {err}"))?;
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
        host.register_font("Noto Sans JP", weight, bundled_font(file))?;
        if real_fonts {
            continue;
        }
        // Documents name Word's fonts, not the bundled one. Aliasing keeps the
        // first render legible rather than silently blank.
        for alias in [
            "Times New Roman",
            "Yu Mincho",
            "MS Mincho",
            "MS Gothic",
            "serif",
        ] {
            host.register_font(alias, weight, bundled_font(file))?;
        }
    }
    if real_fonts {
        for face in jade_docx_native_render::system_fonts::word_document_faces(&bytes, &[]) {
            if let Err(error) = host.register_font_styled(
                &face.family,
                face.weight,
                face.italic,
                &face.path,
                face.index,
            ) {
                eprintln!("skipped {}: {error}", face.family);
            }
        }
    }

    // Taken before the host moves into the renderer, which owns it from then
    // on. The handle stays valid because the pages are shared, not copied.
    let pages = host.pages();

    let mut renderer = CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false)
        .map_err(|_| "renderer setup failed".to_string())?;
    if real_fonts {
        // Equations as the app draws them: Cambria Math's MATH table.
        for face in jade_docx_native_render::system_fonts::word_document_faces(&bytes, &[]) {
            if face.family == jade_docx_native_render::system_fonts::MATH_FAMILY {
                if let Ok(font) = std::fs::read(&face.path) {
                    let _ = renderer.register_math_face(face.family.clone(), font, face.index);
                }
                break;
            }
        }
    }
    renderer
        .render(document, &options)
        .map_err(|_| "render failed".to_string())?;

    let pages = pages.borrow();
    for (index, page) in pages.iter().enumerate() {
        let png = page
            .encode_png()
            .ok_or_else(|| format!("page {}: PNG encoding failed", index + 1))?;
        let path = if index == 0 {
            PathBuf::from(&output)
        } else {
            let base = Path::new(&output);
            let stem = base
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "page".to_string());
            base.with_file_name(format!("{stem}-{}.png", index + 1))
        };
        std::fs::write(&path, &png).map_err(|err| format!("{}: {err}", path.display()))?;
        println!("wrote {} ({} bytes)", path.display(), png.len());
    }
    Ok(())
}
