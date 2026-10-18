//! Cost scenarios for the DOCX engine that count the same on every run.
//!
//! Wall-clock time on a shared CI runner swings tens of percent on unchanged
//! code, so a pull request cannot be judged by it. Instructions executed can:
//! `.github/workflows/perf.yml` runs each scenario here under callgrind, once
//! on the base commit and once on the head, and reports the difference.
//!
//! Each scenario does its setup first and then runs the part it measures
//! inside [`perf_measured_region`]. Callgrind is started with collection off
//! and toggles it on that function, so loading a fixture does not count
//! towards the cost of typing into it.
//!
//! The renderer is the one the browser runs. Text is measured with
//! `jade-docx-layout`'s shaping measurer and real fonts, as the native renderer
//! does; painting goes to [`PerfPainter`], which draws nothing, so what is
//! counted is layout plus the renderer's decisions about what to draw, not a
//! rasteriser.

use futures::executor::block_on;
use jade_docx_editor::canvas_renderer::CanvasRenderer;
use jade_docx_editor::headless_api::apply_headless_commands_to_document;
use jade_docx_editor::render_host::{CanvasRenderHost, ImageCropMapping};
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_docx_layout::font_measurer::RegisteredFontStyle;
use jade_docx_layout::layout_cache::TextMeasurer;
use jade_docx_layout::shaping_measurer::{ShapingMeasurer, ShapingOptions};
use jade_painter::{Painter, TextMetrics};

mod painter;
pub use painter::PerfPainter;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use wasm_bindgen::JsValue;

const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;

/// Keystrokes typed by the `keystroke` scenario. `JADE_PERF_KEYSTROKES`
/// overrides it for a sampling profiler that needs a longer run; the workflow
/// leaves it unset so both sides type the same.
pub const KEYSTROKES: usize = 20;

fn keystrokes() -> usize {
    std::env::var("JADE_PERF_KEYSTROKES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(KEYSTROKES)
}

/// The scenarios, in report order.
pub const SCENARIOS: &[&str] = &["load", "layout", "repaint", "keystroke", "export"];

/// Runs `body` as the measured part of a scenario.
///
/// Callgrind's `--toggle-collect=*perf_measured_region*` keys on this name, so
/// it must stay a real, uninlined function with this name.
#[inline(never)]
pub fn perf_measured_region(body: &mut dyn FnMut()) {
    body();
    std::hint::black_box(());
}

/// The repository root, where the fixtures and fonts live.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn options() -> Options {
    let mut options = Options::default();
    options.show_formatting_marks = false;
    options
}

/// A render host that measures with real fonts and draws nothing. Page geometry is answered for one page size; the bitmap and canvas
/// handles belong to the browser and are never reached.
pub struct PerfHost {
    painter: PerfPainter,
    measurer: Rc<ShapingMeasurer>,
    pages: Rc<Cell<usize>>,
}

impl PerfHost {
    pub fn new() -> Result<Self, String> {
        let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::CHROMIUM_PARITY);
        for (weight, file) in [
            (400, "NotoSansJP-Regular.ttf"),
            (700, "NotoSansJP-Bold.ttf"),
        ] {
            let path = repo_root().join(format!("fonts/Noto_Sans_JP/static/{file}"));
            // The same stand-ins the native renderer's tests register: the
            // fixtures name these families, and the CI runner has none of them.
            for family in [
                "Noto Sans JP",
                "Times New Roman",
                "Yu Mincho",
                "MS Mincho",
                "serif",
            ] {
                measurer
                    .register_font_path(family, weight, RegisteredFontStyle::Normal, &path)
                    .map_err(|err| format!("{}: {err:?}", path.display()))?;
            }
        }
        Ok(Self {
            painter: PerfPainter::default(),
            measurer: Rc::new(measurer),
            pages: Rc::new(Cell::new(1)),
        })
    }
}

impl CanvasRenderHost for PerfHost {
    type Painter = PerfPainter;

    fn painter(&self) -> Self::Painter {
        self.painter.clone()
    }

    fn set_font(&self, font_descriptor: &str) {
        self.painter.set_font(font_descriptor);
    }

    fn draw_image_data_uri(
        &self,
        _uri: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        _crop: Option<ImageCropMapping>,
    ) -> bool {
        self.painter.fill_rect(x, y, width, height);
        true
    }

    fn measure_text_width(&self, text: &str) -> Result<f64, JsValue> {
        Ok(self
            .painter
            .with_font(|font| self.measurer.measure_text(text, font))
            .unwrap_or(0.0))
    }

    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, JsValue> {
        let width = self.measure_text_width(text)?;
        let (font_ascent, font_descent, ink_ascent, ink_descent) = self.painter.with_font(|font| {
            let (ascent, descent) = self.measurer.font_bounding_box(font).unwrap_or((0.0, 0.0));
            let (ink_ascent, ink_descent) = self
                .measurer
                .measure_font_vertical_metrics(font)
                .unwrap_or((ascent, descent));
            (ascent, descent, ink_ascent, ink_descent)
        });
        Ok(TextMetrics {
            width,
            font_bounding_box_ascent: font_ascent,
            font_bounding_box_descent: font_descent,
            actual_bounding_box_ascent: ink_ascent,
            actual_bounding_box_descent: ink_descent,
            actual_bounding_box_right: width,
        })
    }

    fn device_pixel_ratio(&self) -> f64 {
        1.0
    }

    fn update_device_pixel_ratio(&mut self) -> Result<bool, JsValue> {
        Ok(false)
    }

    fn device_to_css(&self, value: f64) -> f64 {
        value
    }

    fn canvas_css_size(&self) -> (f64, f64) {
        (PAGE_WIDTH_PX, PAGE_HEIGHT_PX)
    }

    fn page_count(&self) -> usize {
        self.pages.get()
    }

    fn page_dimensions(&self, _page_index: usize) -> Option<(f64, f64)> {
        Some((PAGE_WIDTH_PX, PAGE_HEIGHT_PX))
    }

    fn reset_pages(&mut self) -> Result<(), JsValue> {
        Ok(())
    }

    fn configure_base_canvas(
        &mut self,
        _width: f64,
        _height: f64,
        _pageless: bool,
        _margin_top: f64,
    ) -> Result<(), JsValue> {
        Ok(())
    }

    fn clear(&mut self, _page_width: f64, _pageless: bool, _full: f64) -> Result<(), JsValue> {
        Ok(())
    }

    fn ensure_pageless_height(&mut self, _height: f64, _margin_bottom: f64) -> Result<(), JsValue> {
        Ok(())
    }

    fn start_new_page(&mut self, _w: f64, _h: f64, _margin_top: f64) -> Result<(), JsValue> {
        self.pages.set(self.pages.get() + 1);
        Ok(())
    }

    fn context(&self) -> &web_sys::CanvasRenderingContext2d {
        unreachable!("perf host: the renderer must not reach for a canvas")
    }

    fn context_mut(&mut self) -> &mut web_sys::CanvasRenderingContext2d {
        unreachable!("perf host: the renderer must not reach for a canvas")
    }

    fn create_page_bitmap(&self, _page_index: usize) -> Result<js_sys::Promise, JsValue> {
        unreachable!("perf host: page bitmaps are a browser-side cache")
    }

    fn take_paged_snapshots(&mut self) -> Result<Option<Vec<JsValue>>, JsValue> {
        Ok(None)
    }

    fn take_paged_snapshots_for_pages(
        &mut self,
        _pages: Option<&[usize]>,
    ) -> Result<Option<Vec<JsValue>>, JsValue> {
        Ok(None)
    }
}

fn load(bytes: &[u8], options: &Options) -> Result<WordDocument, String> {
    block_on(WordDocument::load(bytes, options)).map_err(|err| format!("load: {err:?}"))
}

fn renderer() -> Result<CanvasRenderer<PerfPainter>, String> {
    CanvasRenderer::with_host(Box::new(PerfHost::new()?), false)
        .map_err(|err| format!("renderer: {err:?}"))
}

fn render_cold(
    renderer: &mut CanvasRenderer<PerfPainter>,
    model: &WordDocument,
    options: &Options,
) -> Result<(), String> {
    let document = model.document.as_ref().ok_or("no main document")?;
    renderer
        .render(document, options)
        .map_err(|err| format!("render: {err:?}"))
}

/// A paragraph on page 1, preferring one a few paragraphs in (past a title),
/// so a repaint bounded to the first page actually draws the edit.
fn keystroke_paragraph(renderer: &CanvasRenderer<PerfPainter>) -> Option<usize> {
    (6..64)
        .chain(0..6)
        .find(|&index| renderer.cached_paragraph_start_page(index) == Some(1))
}

/// Runs one scenario on `fixture`. Setup happens outside the measured region.
pub fn run(scenario: &str, fixture: &Path) -> Result<(), String> {
    let bytes = std::fs::read(fixture).map_err(|err| format!("{}: {err}", fixture.display()))?;
    // Built once: the editor keeps its options, and building them parses the
    // shortcut table, which no keystroke pays for.
    let options = &options();
    match scenario {
        "load" => {
            let mut loaded = None;
            perf_measured_region(&mut || loaded = Some(load(&bytes, options)));
            loaded.ok_or("load did not run")??;
        }
        "layout" => {
            let model = load(&bytes, options)?;
            let mut renderer = renderer()?;
            let mut result = Ok(());
            perf_measured_region(&mut || result = render_cold(&mut renderer, &model, options));
            result?;
        }
        "repaint" => {
            // Paint cost with layout already known: a full repaint of a laid
            // out document, which is what a theme or zoom switch runs.
            let model = load(&bytes, options)?;
            let mut renderer = renderer()?;
            render_cold(&mut renderer, &model, options)?;
            let mut result = Ok(());
            perf_measured_region(&mut || {
                renderer.mark_full_repaint();
                result = render_cold(&mut renderer, &model, options);
            });
            result?;
        }
        "keystroke" => {
            let mut model = load(&bytes, options)?;
            let mut renderer = renderer()?;
            render_cold(&mut renderer, &model, options)?;
            let paragraph = keystroke_paragraph(&renderer).ok_or("no paragraph on page 1")?;
            let mut result = Ok(());
            perf_measured_region(&mut || {
                for _ in 0..keystrokes() {
                    result = type_one(&mut renderer, &mut model, paragraph, options);
                    if result.is_err() {
                        return;
                    }
                }
            });
            result?;
        }
        "export" => {
            // An edited document, so the writer serialises the main part
            // instead of copying it through untouched.
            let mut model = load(&bytes, options)?;
            insert_text(&mut model, 0)?;
            let mut exported = Ok(Vec::new());
            perf_measured_region(&mut || {
                exported = model
                    .export_docx()
                    .map_err(|err| format!("export: {err:?}"));
            });
            exported?;
        }
        other => return Err(format!("unknown scenario {other:?}; known: {SCENARIOS:?}")),
    }
    Ok(())
}

fn insert_text(model: &mut WordDocument, paragraph: usize) -> Result<(), String> {
    apply_headless_commands_to_document(
        model,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph,
                offset: 0,
                text: "あ".to_string(),
            }],
        },
    )
    .map(|_| ())
    .map_err(|err| format!("insert text: {err:?}"))
}

/// One keystroke: the edit, then the repaint sequence `canvas_view` runs after
/// it, bounded to the first page as a window showing it would be.
fn type_one(
    renderer: &mut CanvasRenderer<PerfPainter>,
    model: &mut WordDocument,
    paragraph: usize,
    options: &Options,
) -> Result<(), String> {
    const VISIBLE_END_PAGE_1IDX: usize = 1;
    // As `canvas_view` does before every edit: with the renderer still
    // holding the document, the edit would deep-clone it to mutate it.
    renderer.drop_document_cache();
    insert_text(model, paragraph)?;
    let edited = model.document.as_ref().ok_or("no main document")?.clone();
    renderer.prefer_visible_window_seek_once();
    renderer.set_incremental_dirty_from(paragraph);
    renderer.invalidate_paragraph_wrap_cache_from(paragraph);
    renderer.mark_dirty_from_paragraph_to_end_bounded(paragraph, VISIBLE_END_PAGE_1IDX);
    let previous_limit = renderer.layout_page_limit();
    renderer.set_layout_page_limit(Some(VISIBLE_END_PAGE_1IDX));
    let result = renderer
        .render_with_numbering_rc(edited, options, None, None, None, None)
        .map_err(|err| format!("repaint: {err:?}"));
    renderer.set_layout_page_limit(previous_limit);
    result
}
